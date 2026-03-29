/// A bundled app template (README.md + optional Dockerfile).
#[derive(Debug)]
pub struct Template {
    pub readme: &'static str,
    pub dockerfile: Option<&'static str>,
}

/// Returns the template for `name`, or `None` if unrecognized.
/// Matching is case-insensitive; hyphens and underscores are equivalent.
pub fn get(name: &str) -> Option<Template> {
    let key = name.to_lowercase().replace('-', "_");
    match key.as_str() {
        "clickhouse" => Some(Template {
            readme: include_str!("../app/clickhouse/README.md"),
            dockerfile: Some(include_str!("../app/clickhouse/Dockerfile")),
        }),
        "duckdb" => Some(Template {
            readme: include_str!("../app/duckdb/README.md"),
            dockerfile: Some(include_str!("../app/duckdb/Dockerfile")),
        }),
        "elasticsearch" => Some(Template {
            readme: include_str!("../app/elasticsearch/README.md"),
            dockerfile: None,
        }),
        "mongodb" => Some(Template {
            readme: include_str!("../app/mongodb/README.md"),
            dockerfile: Some(include_str!("../app/mongodb/Dockerfile")),
        }),
        "mssql" => Some(Template {
            readme: include_str!("../app/mssql/README.md"),
            dockerfile: Some(include_str!("../app/mssql/Dockerfile")),
        }),
        "mysql" => Some(Template {
            readme: include_str!("../app/mysql/README.md"),
            dockerfile: Some(include_str!("../app/mysql/Dockerfile")),
        }),
        "pgvector" => Some(Template {
            readme: include_str!("../app/pgvector/README.md"),
            dockerfile: Some(include_str!("../app/pgvector/Dockerfile")),
        }),
        "postgresql" => Some(Template {
            readme: include_str!("../app/postgresql/README.md"),
            dockerfile: Some(include_str!("../app/postgresql/Dockerfile")),
        }),
        "qdrant" => Some(Template {
            readme: include_str!("../app/qdrant/README.md"),
            dockerfile: None,
        }),
        "redis" => Some(Template {
            readme: include_str!("../app/redis/README.md"),
            dockerfile: Some(include_str!("../app/redis/Dockerfile")),
        }),
        "snowflake" => Some(Template {
            readme: include_str!("../app/snowflake/README.md"),
            dockerfile: Some(include_str!("../app/snowflake/Dockerfile")),
        }),
        "sqlite" => Some(Template {
            readme: include_str!("../app/sqlite/README.md"),
            dockerfile: Some(include_str!("../app/sqlite/Dockerfile")),
        }),
        "vectorless_rag" => Some(Template {
            readme: include_str!("../app/vectorless_rag/README.md"),
            dockerfile: None,
        }),
        "weaviate" => Some(Template {
            readme: include_str!("../app/weaviate/README.md"),
            dockerfile: None,
        }),
        _ => None,
    }
}

/// Canonical template slugs (as accepted by `get`).
pub const NAMES: &[&str] = &[
    "clickhouse",
    "duckdb",
    "elasticsearch",
    "mongodb",
    "mssql",
    "mysql",
    "pgvector",
    "postgresql",
    "qdrant",
    "redis",
    "snowflake",
    "sqlite",
    "vectorless-rag",
    "weaviate",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_named_templates_resolve() {
        for name in NAMES {
            assert!(get(name).is_some(), "Template '{name}' listed in NAMES but not found");
        }
    }

    #[test]
    fn unknown_template_returns_none() {
        assert!(get("nonexistent").is_none());
    }

    #[test]
    fn hyphen_and_underscore_both_work() {
        assert!(get("vectorless-rag").is_some());
        assert!(get("vectorless_rag").is_some());
    }
}
