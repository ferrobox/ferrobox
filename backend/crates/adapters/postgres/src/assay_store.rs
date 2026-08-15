use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ferrobox_domain::assay::{
    Assay, AssayComponent, AssayComponentKind, AssayFinding, AssaySeverity, AssayStatus,
};
use ferrobox_domain::ids::{AssayId, RepositoryId};
use ferrobox_domain::package_coordinate::{PackageCoordinate, PackageName, PackageVersion};
use ferrobox_ports::assay_store::{AssayStore, AssayStoreError};
use serde_json::{Value, json};
use sqlx::Row;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, query};
use thiserror::Error;
use uuid::Uuid;

use crate::ecosystem_column;

/// Adaptador de [`AssayStore`] contra `PostgreSQL`.
pub struct PostgresAssayStore {
    pool: PgPool,
}

impl PostgresAssayStore {
    /// Construye el adaptador a partir de un `pool` de conexiones.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct RowConversionError(String);

fn backend_error(message: impl Into<String>) -> AssayStoreError {
    AssayStoreError::Backend(Box::new(RowConversionError(message.into())))
}

fn components_json(components: &[AssayComponent]) -> Value {
    Value::Array(
        components
            .iter()
            .map(|component| {
                json!({
                    "name": component.name(),
                    "version": component.version(),
                    "purl": component.purl(),
                    "kind": component.kind().as_str(),
                })
            })
            .collect(),
    )
}

fn findings_json(findings: &[AssayFinding]) -> Value {
    Value::Array(
        findings
            .iter()
            .map(|finding| {
                json!({
                    "vulnerability_id": finding.vulnerability_id(),
                    "aliases": finding.aliases(),
                    "title": finding.title(),
                    "severity": finding.severity().as_str(),
                    "component_name": finding.component_name(),
                    "component_version": finding.component_version(),
                    "fixed_version": finding.fixed_version(),
                    "details_url": finding.details_url(),
                })
            })
            .collect(),
    )
}

fn parse_components(value: Value) -> Vec<AssayComponent> {
    let Value::Array(items) = value else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?.to_string();
            let version = item.get("version")?.as_str()?.to_string();
            let purl = item
                .get("purl")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let kind = AssayComponentKind::parse(item.get("kind").and_then(Value::as_str).unwrap_or("direct"));
            Some(AssayComponent::new(name, version, purl, kind))
        })
        .collect()
}

fn parse_findings(value: Value) -> Vec<AssayFinding> {
    let Value::Array(items) = value else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| {
            let vulnerability_id = item.get("vulnerability_id")?.as_str()?.to_string();
            let aliases = item
                .get("aliases")
                .and_then(Value::as_array)
                .map(|aliases| {
                    aliases
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let title = item.get("title").and_then(Value::as_str).unwrap_or("").to_string();
            let severity = AssaySeverity::parse(
                item.get("severity").and_then(Value::as_str).unwrap_or("unknown"),
            );
            let component_name = item.get("component_name")?.as_str()?.to_string();
            let component_version = item.get("component_version")?.as_str()?.to_string();
            let fixed_version = item
                .get("fixed_version")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let details_url = item
                .get("details_url")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            Some(AssayFinding::new(
                vulnerability_id,
                aliases,
                title,
                severity,
                component_name,
                component_version,
                fixed_version,
                details_url,
            ))
        })
        .collect()
}

fn row_to_assay(row: &PgRow) -> Result<Assay, AssayStoreError> {
    let id: Uuid = row.try_get("id").map_err(|err| backend_error(err.to_string()))?;
    let repository_id: Uuid = row
        .try_get("repository_id")
        .map_err(|err| backend_error(err.to_string()))?;
    let ecosystem: String = row
        .try_get("ecosystem")
        .map_err(|err| backend_error(err.to_string()))?;
    let package_name: String = row
        .try_get("package_name")
        .map_err(|err| backend_error(err.to_string()))?;
    let package_version: String = row
        .try_get("package_version")
        .map_err(|err| backend_error(err.to_string()))?;
    let status: String = row
        .try_get("status")
        .map_err(|err| backend_error(err.to_string()))?;
    let error_message: Option<String> = row
        .try_get("error_message")
        .map_err(|err| backend_error(err.to_string()))?;
    let scanned_at: Option<DateTime<Utc>> = row
        .try_get("scanned_at")
        .map_err(|err| backend_error(err.to_string()))?;
    let components: Value = row
        .try_get("components")
        .map_err(|err| backend_error(err.to_string()))?;
    let findings: Value = row
        .try_get("findings")
        .map_err(|err| backend_error(err.to_string()))?;

    let ecosystem = ecosystem_column::from_column(&ecosystem).map_err(backend_error)?;
    let name = PackageName::parse(package_name).map_err(|err| backend_error(err.to_string()))?;
    let version =
        PackageVersion::parse(package_version).map_err(|err| backend_error(err.to_string()))?;
    let status = AssayStatus::parse(&status)
        .ok_or_else(|| backend_error(format!("unknown assay status: {status}")))?;

    Ok(Assay::from_parts(
        AssayId::from(id),
        RepositoryId::from(repository_id),
        PackageCoordinate::new(ecosystem, name, version),
        status,
        scanned_at.map(|stamp| stamp.to_rfc3339()),
        error_message,
        parse_components(components),
        parse_findings(findings),
    ))
}

#[async_trait]
impl AssayStore for PostgresAssayStore {
    async fn upsert(&self, assay: &Assay) -> Result<(), AssayStoreError> {
        let id: Uuid = assay.id().into();
        let repository_id: Uuid = assay.repository_id().into();
        let counts = assay.counts();
        let scanned_at = assay.scanned_at().and_then(|value| {
            DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|stamp| stamp.with_timezone(&Utc))
        });
        let component_count = i32::try_from(assay.components().len())
            .map_err(|err| backend_error(err.to_string()))?;

        query(
            r"
            INSERT INTO assays (
                id, repository_id, ecosystem, package_name, package_version,
                status, error_message, scanned_at,
                critical_count, high_count, medium_count, low_count, unknown_count,
                component_count, components, findings
            )
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
            ON CONFLICT (repository_id, ecosystem, package_name, package_version) DO UPDATE
            SET id = EXCLUDED.id,
                status = EXCLUDED.status,
                error_message = EXCLUDED.error_message,
                scanned_at = EXCLUDED.scanned_at,
                critical_count = EXCLUDED.critical_count,
                high_count = EXCLUDED.high_count,
                medium_count = EXCLUDED.medium_count,
                low_count = EXCLUDED.low_count,
                unknown_count = EXCLUDED.unknown_count,
                component_count = EXCLUDED.component_count,
                components = EXCLUDED.components,
                findings = EXCLUDED.findings
            ",
        )
        .bind(id)
        .bind(repository_id)
        .bind(ecosystem_column::to_column(assay.coordinate().ecosystem()))
        .bind(assay.coordinate().name().as_str())
        .bind(assay.coordinate().version().as_str())
        .bind(assay.status().as_str())
        .bind(assay.error_message())
        .bind(scanned_at)
        .bind(i32::try_from(counts.critical).unwrap_or(i32::MAX))
        .bind(i32::try_from(counts.high).unwrap_or(i32::MAX))
        .bind(i32::try_from(counts.medium).unwrap_or(i32::MAX))
        .bind(i32::try_from(counts.low).unwrap_or(i32::MAX))
        .bind(i32::try_from(counts.unknown).unwrap_or(i32::MAX))
        .bind(component_count)
        .bind(components_json(assay.components()))
        .bind(findings_json(assay.findings()))
        .execute(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;

        Ok(())
    }

    async fn find_by_id(&self, id: AssayId) -> Result<Option<Assay>, AssayStoreError> {
        let id: Uuid = id.into();
        let row = query("SELECT * FROM assays WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| backend_error(err.to_string()))?;
        row.map(|row| row_to_assay(&row)).transpose()
    }

    async fn find_by_coordinate(
        &self,
        repository_id: RepositoryId,
        coordinate: &PackageCoordinate,
    ) -> Result<Option<Assay>, AssayStoreError> {
        let repository_id: Uuid = repository_id.into();
        let row = query(
            r"
            SELECT * FROM assays
            WHERE repository_id = $1
              AND ecosystem = $2
              AND lower(package_name) = lower($3)
              AND package_version = $4
            ",
        )
        .bind(repository_id)
        .bind(ecosystem_column::to_column(coordinate.ecosystem()))
        .bind(coordinate.name().as_str())
        .bind(coordinate.version().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| backend_error(err.to_string()))?;
        row.map(|row| row_to_assay(&row)).transpose()
    }

    async fn find_by_repository(
        &self,
        repository_id: RepositoryId,
    ) -> Result<Vec<Assay>, AssayStoreError> {
        let repository_id: Uuid = repository_id.into();
        let rows = query("SELECT * FROM assays WHERE repository_id = $1 ORDER BY scanned_at DESC NULLS LAST")
            .bind(repository_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|err| backend_error(err.to_string()))?;
        rows.iter().map(row_to_assay).collect()
    }

    async fn find_all(&self) -> Result<Vec<Assay>, AssayStoreError> {
        let rows = query("SELECT * FROM assays ORDER BY scanned_at DESC NULLS LAST")
            .fetch_all(&self.pool)
            .await
            .map_err(|err| backend_error(err.to_string()))?;
        rows.iter().map(row_to_assay).collect()
    }
}
