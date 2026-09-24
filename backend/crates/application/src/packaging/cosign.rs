//! Detección de accesorios OCI de firma (Cosign / Sigstore, Notation)
//! y de atestaciones (DSSE / in-toto).
//!
//! Cosign puede colgar la firma de dos formas:
//!
//! * etiqueta clásica `sha256-<digest>.sig` en el mismo nombre de imagen;
//! * manifiesto con `subject` y el [Referrers API](https://github.com/opencontainers/distribution-spec)
//!   (`GET /v2/<name>/referrers/<digest>`).
//!
//! Esta capa no verifica criptográficamente: solo identifica y enlaza
//! el accesorio con el digest firmado para mostrarlo y servirlo.

/// Media type del payload de firma simple de Cosign.
pub const COSIGN_SIMPLE_MEDIA_TYPE: &str = "application/vnd.dev.cosign.simplesigning.v1+json";

/// Media type de un sobre DSSE (atestaciones Cosign).
pub const DSSE_ENVELOPE_MEDIA_TYPE: &str = "application/vnd.dsse.envelope.v1+json";

/// Media type de una firma Notation / Notary.
pub const NOTATION_MEDIA_TYPE: &str = "application/vnd.cncf.notary.signature";

/// Clase de accesorio OCI asociado a un artefacto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessoryKind {
    /// Firma Cosign / Sigstore / Notation.
    Signature,
    /// Atestación (DSSE, in-toto).
    Attestation,
    /// Otro referrer (SBOM adjunto, etc.).
    Other,
}

impl AccessoryKind {
    /// Etiqueta persistida en la entrada de índice.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Signature => "signature",
            Self::Attestation => "attestation",
            Self::Other => "other",
        }
    }

    /// Reconstruye la clase desde la etiqueta persistida.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "signature" => Some(Self::Signature),
            "attestation" => Some(Self::Attestation),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// Metadatos de accesorio extraídos de un manifiesto OCI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessoryMeta {
    /// Clase detectada. `None` si el manifiesto es una imagen normal.
    pub kind: Option<AccessoryKind>,
    /// Digest `sha256:…` del artefacto sujeto, si se conoce.
    pub subject: Option<String>,
    /// `artifactType` OCI, si el manifiesto o las capas lo declaran.
    pub artifact_type: Option<String>,
}

/// Inspecciona un manifiesto publicado y decide si es un accesorio.
#[must_use]
pub fn inspect(reference: &str, media_type: &str, body: &[u8]) -> AccessoryMeta {
    let parsed = serde_json::from_slice::<serde_json::Value>(body).ok();
    let subject = parsed
        .as_ref()
        .and_then(subject_from_manifest)
        .or_else(|| subject_from_tag(reference));
    let artifact_type = parsed.as_ref().and_then(artifact_type_from_manifest);
    let layer_types = parsed.as_ref().map(layer_media_types).unwrap_or_default();
    let kind = classify(
        reference,
        media_type,
        artifact_type.as_deref(),
        &layer_types,
        subject.is_some(),
    );
    let artifact_type = artifact_type.or_else(|| {
        kind.and_then(|kind| match kind {
            AccessoryKind::Signature => Some(COSIGN_SIMPLE_MEDIA_TYPE.to_string()),
            AccessoryKind::Attestation => Some(DSSE_ENVELOPE_MEDIA_TYPE.to_string()),
            AccessoryKind::Other => None,
        })
    });
    AccessoryMeta {
        kind,
        subject,
        artifact_type,
    }
}

/// `true` si la referencia es un digest `sha256:…`.
#[must_use]
pub fn is_digest_reference(value: &str) -> bool {
    value.to_ascii_lowercase().starts_with("sha256:")
}

/// `true` si la etiqueta es un accesorio Cosign (`.sig`, `.att`, `.sbom`).
#[must_use]
pub fn is_accessory_tag(reference: &str) -> bool {
    has_extension(reference, "sig")
        || has_extension(reference, "att")
        || has_extension(reference, "sbom")
}

/// `true` si la etiqueta es específicamente una firma Cosign (`.sig`).
#[must_use]
pub fn is_signature_tag(reference: &str) -> bool {
    has_extension(reference, "sig")
}

/// Oculta blobs, digest-only y accesorios del listado de la UI.
#[must_use]
pub fn should_hide_from_listing(name: &str, reference: &str, accessory: Option<&str>) -> bool {
    name == "_blob"
        || is_digest_reference(reference)
        || accessory.is_some()
        || is_accessory_tag(reference)
}

/// `true` si este accesorio cuenta como firma para el badge.
#[must_use]
pub fn is_signature_accessory(accessory: Option<&str>, reference: &str) -> bool {
    matches!(accessory, Some("signature")) || is_signature_tag(reference)
}

/// Sujeto firmado: campo `subject` o convención de etiqueta Cosign.
#[must_use]
pub fn signature_subject(subject: Option<&str>, reference: &str) -> Option<String> {
    subject
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .or_else(|| subject_from_tag(reference))
}

/// `true` si `digest` (`sha256:hex`) corresponde al checksum del artefacto.
#[must_use]
pub fn digest_matches_checksum(digest: &str, checksum: &str) -> bool {
    let expected = format!("sha256:{checksum}");
    digest.eq_ignore_ascii_case(&expected)
}

/// Extrae el digest sujeto de una etiqueta `sha256-<hex>.sig` (o `.att` / `.sbom`).
#[must_use]
pub fn subject_from_tag(reference: &str) -> Option<String> {
    let lower = reference.trim().to_ascii_lowercase();
    let stem = lower
        .strip_suffix(".sig")
        .or_else(|| lower.strip_suffix(".att"))
        .or_else(|| lower.strip_suffix(".sbom"))?;
    let hex = stem.strip_prefix("sha256-")?;
    if hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Some(format!("sha256:{hex}"))
    } else {
        None
    }
}

fn subject_from_manifest(value: &serde_json::Value) -> Option<String> {
    value
        .get("subject")
        .and_then(|subject| subject.get("digest"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|digest| is_digest_reference(digest))
        .map(str::to_ascii_lowercase)
}

fn artifact_type_from_manifest(value: &serde_json::Value) -> Option<String> {
    value
        .get("artifactType")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            layer_media_types(value)
                .into_iter()
                .find(|media| is_known_accessory_media(media))
        })
}

fn layer_media_types(value: &serde_json::Value) -> Vec<String> {
    let mut types = Vec::new();
    for key in ["layers", "blobs"] {
        let Some(items) = value.get(key).and_then(serde_json::Value::as_array) else {
            continue;
        };
        for item in items {
            if let Some(media) = item
                .get("mediaType")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                types.push(media.to_string());
            }
        }
    }
    types
}

fn classify(
    reference: &str,
    media_type: &str,
    artifact_type: Option<&str>,
    layer_types: &[String],
    has_subject: bool,
) -> Option<AccessoryKind> {
    if is_signature_media(media_type)
        || artifact_type.is_some_and(is_signature_media)
        || layer_types.iter().any(|media| is_signature_media(media))
        || is_signature_tag(reference)
    {
        return Some(AccessoryKind::Signature);
    }
    if is_attestation_media(media_type)
        || artifact_type.is_some_and(is_attestation_media)
        || layer_types.iter().any(|media| is_attestation_media(media))
        || has_extension(reference, "att")
    {
        return Some(AccessoryKind::Attestation);
    }
    if is_accessory_tag(reference) || has_subject {
        return Some(AccessoryKind::Other);
    }
    None
}

fn is_signature_media(media_type: &str) -> bool {
    matches!(
        media_type,
        COSIGN_SIMPLE_MEDIA_TYPE
            | NOTATION_MEDIA_TYPE
            | "application/vnd.dev.sigstore.bundle+json"
            | "application/vnd.dev.sigstore.bundle.v0.3+json"
    )
}

fn is_attestation_media(media_type: &str) -> bool {
    matches!(
        media_type,
        DSSE_ENVELOPE_MEDIA_TYPE | "application/vnd.in-toto+json"
    )
}

fn is_known_accessory_media(media_type: &str) -> bool {
    is_signature_media(media_type) || is_attestation_media(media_type)
}

fn has_extension(value: &str, extension: &str) -> bool {
    std::path::Path::new(value)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_from_classic_cosign_tag() {
        let digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let tag = digest.replacen(':', "-", 1) + ".sig";
        assert_eq!(subject_from_tag(&tag).as_deref(), Some(digest));
        assert!(is_signature_tag(&tag));
        assert!(should_hide_from_listing("demo", &tag, None));
    }

    #[test]
    fn inspects_referrer_manifest_with_subject() {
        let body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "artifactType": COSIGN_SIMPLE_MEDIA_TYPE,
            "config": {
                "mediaType": "application/vnd.oci.empty.v1+json",
                "digest": "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
                "size": 2
            },
            "layers": [{
                "mediaType": COSIGN_SIMPLE_MEDIA_TYPE,
                "digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "size": 32
            }],
            "subject": {
                "mediaType": "application/vnd.oci.image.manifest.v1+json",
                "digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "size": 100
            }
        });
        let meta = inspect(
            "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "application/vnd.oci.image.manifest.v1+json",
            &serde_json::to_vec(&body).unwrap(),
        );
        assert_eq!(meta.kind, Some(AccessoryKind::Signature));
        assert_eq!(
            meta.subject.as_deref(),
            Some("sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
        );
        assert_eq!(
            meta.artifact_type.as_deref(),
            Some(COSIGN_SIMPLE_MEDIA_TYPE)
        );
    }

    #[test]
    fn regular_image_is_not_an_accessory() {
        let body = serde_json::json!({
            "schemaVersion": 2,
            "config": { "digest": "sha256:1" },
            "layers": [{ "digest": "sha256:2" }]
        });
        let meta = inspect(
            "latest",
            "application/vnd.oci.image.manifest.v1+json",
            &serde_json::to_vec(&body).unwrap(),
        );
        assert_eq!(meta.kind, None);
        assert_eq!(meta.subject, None);
    }

    #[test]
    fn digest_matches_artifact_checksum() {
        let checksum = "aa".repeat(32);
        assert!(digest_matches_checksum(
            &format!("sha256:{checksum}"),
            &checksum
        ));
        assert!(!digest_matches_checksum("sha256:deadbeef", &checksum));
    }
}
