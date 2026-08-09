use url::Url;

use crate::ids::RepositoryId;

/// La estrategia de origen y almacenamiento de un repositorio.
///
/// A diferencia de una enumeración al estilo C (una simple lista de
/// etiquetas), cada variante de este tipo lleva datos propios y
/// distintos -- es un *tipo algebraico de datos*: el compilador conoce,
/// para cada variante, exactamente qué campos existen, y exige manejar
/// todas las variantes explícitamente en cualquier `match` sobre este
/// tipo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryKind {
    /// Almacenamiento propio: `FerroBox` es la fuente de verdad del
    /// contenido publicado directamente aquí.
    Forge,

    /// Réplica cacheada de una fuente externa. La primera petición de un
    /// artefacto ausente se resuelve contra `upstream`; las peticiones
    /// posteriores se sirven desde la copia ya almacenada localmente.
    Mirror {
        /// URL base del repositorio externo que se está replicando.
        upstream: Url,
    },

    /// Punto de acceso único que agrega varios repositorios (`Forge`
    /// y/o `Mirror`) bajo una sola URL, resolviendo internamente contra
    /// cuál de ellos responder.
    Alloy {
        /// Repositorios agregados, en el orden en que se consultan.
        members: Vec<RepositoryId>,
    },
}

impl RepositoryKind {
    /// Descripción breve en una palabra, útil para registros y depuración.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Forge => "forge",
            Self::Mirror { .. } => "mirror",
            Self::Alloy { .. } => "alloy",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_instances_are_always_equal() {
        assert_eq!(RepositoryKind::Forge, RepositoryKind::Forge);
    }

    #[test]
    fn mirrors_are_equal_only_if_they_point_to_the_same_upstream() {
        let a = RepositoryKind::Mirror {
            upstream: Url::parse("https://crates.io").unwrap(),
        };
        let b = RepositoryKind::Mirror {
            upstream: Url::parse("https://crates.io").unwrap(),
        };
        let c = RepositoryKind::Mirror {
            upstream: Url::parse("https://registry.npmjs.org").unwrap(),
        };

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn label_identifies_each_variant() {
        assert_eq!(RepositoryKind::Forge.label(), "forge");
        assert_eq!(
            RepositoryKind::Mirror {
                upstream: Url::parse("https://crates.io").unwrap()
            }
            .label(),
            "mirror"
        );
        assert_eq!(
            RepositoryKind::Alloy {
                members: vec![RepositoryId::new()],
            }
            .label(),
            "alloy"
        );
    }
}
