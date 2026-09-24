//! Ensaye de un paquete: composición (inventario) e impurezas
//! (vulnerabilidades conocidas).

use crate::ids::{AssayId, RepositoryId};
use crate::package_coordinate::PackageCoordinate;

/// Resultado de un ensaye.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssayStatus {
    /// Inventario y consulta de vulnerabilidades completados.
    Ready,
    /// El inventario se obtuvo, pero falló la consulta de vulnerabilidades.
    Failed,
    /// Este ecosistema todavía no admite ensaye.
    Unsupported,
}

impl AssayStatus {
    /// Etiqueta estable usada en persistencia y en la API HTTP.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
        }
    }

    /// Parsea la etiqueta estable.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ready" => Some(Self::Ready),
            "failed" => Some(Self::Failed),
            "unsupported" => Some(Self::Unsupported),
            _ => None,
        }
    }
}

/// Severidad de un hallazgo, alineada con la escala habitual de
/// puntuación CVSS (*Common Vulnerability Scoring System*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssaySeverity {
    /// 9.0 o más, o etiqueta `CRITICAL`.
    Critical,
    /// 7.0–8.9, o etiqueta `HIGH`.
    High,
    /// 4.0–6.9, o etiqueta `MEDIUM` / `MODERATE`.
    Medium,
    /// 0.1–3.9, o etiqueta `LOW`.
    Low,
    /// Sin puntuación conocida.
    Unknown,
}

impl AssaySeverity {
    /// Etiqueta estable usada en persistencia y en la API HTTP.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Unknown => "unknown",
        }
    }

    /// Parsea la etiqueta estable o una etiqueta de asesoría (`CRITICAL`…).
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "critical" => Self::Critical,
            "high" => Self::High,
            "medium" | "moderate" => Self::Medium,
            "low" => Self::Low,
            _ => Self::Unknown,
        }
    }

    /// Traduce una puntuación CVSS 3.x (0–10) a severidad.
    #[must_use]
    pub fn from_cvss_score(score: f64) -> Self {
        if score >= 9.0 {
            Self::Critical
        } else if score >= 7.0 {
            Self::High
        } else if score >= 4.0 {
            Self::Medium
        } else if score > 0.0 {
            Self::Low
        } else {
            Self::Unknown
        }
    }
}

/// Papel de un componente en el inventario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssayComponentKind {
    /// El paquete ensayado.
    Root,
    /// Dependencia declarada de primer nivel.
    Direct,
    /// Dependencia resuelta desde un lockfile (no declarada en el manifiesto).
    Transitive,
}

impl AssayComponentKind {
    /// Etiqueta estable.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Direct => "direct",
            Self::Transitive => "transitive",
        }
    }

    /// Parsea la etiqueta estable.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "root" => Self::Root,
            "transitive" => Self::Transitive,
            _ => Self::Direct,
        }
    }
}

/// Un componente del inventario del ensaye.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssayComponent {
    name: String,
    version: String,
    purl: Option<String>,
    kind: AssayComponentKind,
    licenses: Vec<String>,
}

impl AssayComponent {
    /// Construye un componente del inventario.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        purl: Option<String>,
        kind: AssayComponentKind,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            purl,
            kind,
            licenses: Vec::new(),
        }
    }

    /// Añade licencias declaradas, sin duplicar.
    pub fn add_licenses(&mut self, licenses: impl IntoIterator<Item = String>) {
        for license in licenses {
            let license = license.trim();
            if license.is_empty() {
                continue;
            }
            if !self
                .licenses
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(license))
            {
                self.licenses.push(license.to_string());
            }
        }
    }

    /// Sustituye las licencias declaradas.
    #[must_use]
    pub fn with_licenses(mut self, licenses: Vec<String>) -> Self {
        self.licenses.clear();
        self.add_licenses(licenses);
        self
    }

    /// Nombre del componente.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Versión o rango declarado.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Identificador `purl` (*package URL*), si se pudo construir.
    #[must_use]
    pub fn purl(&self) -> Option<&str> {
        self.purl.as_deref()
    }

    /// Papel en el inventario.
    #[must_use]
    pub fn kind(&self) -> AssayComponentKind {
        self.kind
    }

    /// Licencias declaradas (SPDX u otras etiquetas del manifiesto).
    #[must_use]
    pub fn licenses(&self) -> &[String] {
        &self.licenses
    }
}

/// Una impureza: vulnerabilidad conocida que afecta a un componente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssayFinding {
    vulnerability_id: String,
    aliases: Vec<String>,
    title: String,
    severity: AssaySeverity,
    component_name: String,
    component_version: String,
    fixed_version: Option<String>,
    details_url: Option<String>,
}

impl AssayFinding {
    /// Construye un hallazgo.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        vulnerability_id: impl Into<String>,
        aliases: Vec<String>,
        title: impl Into<String>,
        severity: AssaySeverity,
        component_name: impl Into<String>,
        component_version: impl Into<String>,
        fixed_version: Option<String>,
        details_url: Option<String>,
    ) -> Self {
        Self {
            vulnerability_id: vulnerability_id.into(),
            aliases,
            title: title.into(),
            severity,
            component_name: component_name.into(),
            component_version: component_version.into(),
            fixed_version,
            details_url,
        }
    }

    /// Identificador principal (`CVE-…`, `GHSA-…`, `RUSTSEC-…`).
    #[must_use]
    pub fn vulnerability_id(&self) -> &str {
        &self.vulnerability_id
    }

    /// Otros identificadores equivalentes.
    #[must_use]
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }

    /// Título corto.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Severidad.
    #[must_use]
    pub fn severity(&self) -> AssaySeverity {
        self.severity
    }

    /// Componente afectado.
    #[must_use]
    pub fn component_name(&self) -> &str {
        &self.component_name
    }

    /// Versión del componente afectado.
    #[must_use]
    pub fn component_version(&self) -> &str {
        &self.component_version
    }

    /// Primera versión que corrige el hallazgo, si se conoce.
    #[must_use]
    pub fn fixed_version(&self) -> Option<&str> {
        self.fixed_version.as_deref()
    }

    /// Enlace a la ficha pública (NVD, GitHub Advisory, …).
    #[must_use]
    pub fn details_url(&self) -> Option<&str> {
        self.details_url.as_deref()
    }
}

/// Recuento de hallazgos por severidad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AssayCounts {
    /// Críticos.
    pub critical: u32,
    /// Altos.
    pub high: u32,
    /// Medios.
    pub medium: u32,
    /// Bajos.
    pub low: u32,
    /// Sin puntuación.
    pub unknown: u32,
}

impl AssayCounts {
    /// Suma de todos los hallazgos.
    #[must_use]
    pub fn total(self) -> u32 {
        self.critical
            .saturating_add(self.high)
            .saturating_add(self.medium)
            .saturating_add(self.low)
            .saturating_add(self.unknown)
    }
}

/// Ensaye de una versión de paquete en un repositorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assay {
    id: AssayId,
    repository_id: RepositoryId,
    coordinate: PackageCoordinate,
    status: AssayStatus,
    scanned_at: Option<String>,
    error_message: Option<String>,
    components: Vec<AssayComponent>,
    findings: Vec<AssayFinding>,
}

impl Assay {
    /// Construye un ensaye completo.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn from_parts(
        id: AssayId,
        repository_id: RepositoryId,
        coordinate: PackageCoordinate,
        status: AssayStatus,
        scanned_at: Option<String>,
        error_message: Option<String>,
        components: Vec<AssayComponent>,
        findings: Vec<AssayFinding>,
    ) -> Self {
        Self {
            id,
            repository_id,
            coordinate,
            status,
            scanned_at,
            error_message,
            components,
            findings,
        }
    }

    /// Identificador del ensaye.
    #[must_use]
    pub fn id(&self) -> AssayId {
        self.id
    }

    /// Repositorio que almacena el paquete ensayado.
    #[must_use]
    pub fn repository_id(&self) -> RepositoryId {
        self.repository_id
    }

    /// Coordenada ensayada.
    #[must_use]
    pub fn coordinate(&self) -> &PackageCoordinate {
        &self.coordinate
    }

    /// Estado del ensaye.
    #[must_use]
    pub fn status(&self) -> AssayStatus {
        self.status
    }

    /// Instante RFC 3339 del último ensaye, si lo hay.
    #[must_use]
    pub fn scanned_at(&self) -> Option<&str> {
        self.scanned_at.as_deref()
    }

    /// Detalle si el ensaye falló o no aplica.
    #[must_use]
    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// Inventario de componentes.
    #[must_use]
    pub fn components(&self) -> &[AssayComponent] {
        &self.components
    }

    /// Hallazgos (impurezas).
    #[must_use]
    pub fn findings(&self) -> &[AssayFinding] {
        &self.findings
    }

    /// Recuento por severidad.
    #[must_use]
    pub fn counts(&self) -> AssayCounts {
        let mut counts = AssayCounts::default();
        for finding in &self.findings {
            match finding.severity {
                AssaySeverity::Critical => counts.critical += 1,
                AssaySeverity::High => counts.high += 1,
                AssaySeverity::Medium => counts.medium += 1,
                AssaySeverity::Low => counts.low += 1,
                AssaySeverity::Unknown => counts.unknown += 1,
            }
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_coordinate::{PackageEcosystem, PackageName, PackageVersion};

    #[test]
    fn counts_group_findings_by_severity() {
        let assay = Assay::from_parts(
            AssayId::new(),
            RepositoryId::new(),
            PackageCoordinate::new(
                PackageEcosystem::Npm,
                PackageName::parse("lodash").unwrap(),
                PackageVersion::parse("4.17.20").unwrap(),
            ),
            AssayStatus::Ready,
            None,
            None,
            Vec::new(),
            vec![
                AssayFinding::new(
                    "CVE-1",
                    Vec::new(),
                    "a",
                    AssaySeverity::Critical,
                    "lodash",
                    "4.17.20",
                    None,
                    None,
                ),
                AssayFinding::new(
                    "CVE-2",
                    Vec::new(),
                    "b",
                    AssaySeverity::High,
                    "lodash",
                    "4.17.20",
                    None,
                    None,
                ),
            ],
        );
        let counts = assay.counts();
        assert_eq!(counts.critical, 1);
        assert_eq!(counts.high, 1);
        assert_eq!(counts.total(), 2);
    }

    #[test]
    fn component_licenses_deduplicate_case_insensitively() {
        let mut component = AssayComponent::new("demo", "1.0.0", None, AssayComponentKind::Root);
        component.add_licenses(["MIT".to_string(), "mit".to_string(), "Apache-2.0".to_string()]);
        assert_eq!(component.licenses(), &["MIT".to_string(), "Apache-2.0".to_string()]);
    }

    #[test]
    fn cvss_score_maps_to_severity_bands() {
        assert_eq!(AssaySeverity::from_cvss_score(9.8), AssaySeverity::Critical);
        assert_eq!(AssaySeverity::from_cvss_score(7.5), AssaySeverity::High);
        assert_eq!(AssaySeverity::from_cvss_score(5.0), AssaySeverity::Medium);
        assert_eq!(AssaySeverity::from_cvss_score(2.0), AssaySeverity::Low);
    }
}
