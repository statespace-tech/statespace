//! Image references. Statespace names images by digest, such as
//! `ghcr.io/acme/shop@sha256:…`, so they run exact bytes.

/// Whether `image` names a repository and a digest.
pub fn is_digest(image: &str) -> bool {
    image
        .split_once("@sha256:")
        .is_some_and(|(repository, digest)| {
            !repository.is_empty()
                && repository.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
                })
                && digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

/// The repository of a reference: everything before its tag or digest.
pub fn repository(image: &str) -> &str {
    let without_digest = image.split('@').next().unwrap_or(image);
    match without_digest.rsplit_once(':') {
        // A colon after the last slash starts a tag; before it, a registry port.
        Some((repository, tag)) if !tag.contains('/') => repository,
        _ => without_digest,
    }
}

/// Docker Hub names a repository several ways.
pub fn normalize(repository: &str) -> String {
    let repository = repository
        .strip_prefix("docker.io/")
        .or_else(|| repository.strip_prefix("index.docker.io/"))
        .unwrap_or(repository);
    repository
        .strip_prefix("library/")
        .unwrap_or(repository)
        .to_owned()
}

/// `repository@sha256:…` from a container runtime's image ID, which may
/// carry a scheme such as `docker-pullable://`.
pub fn digest_reference(image_id: &str) -> Option<String> {
    let reference = image_id.rsplit("://").next().unwrap_or(image_id);
    is_digest(reference).then(|| reference.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_digests() {
        let digest = format!("ghcr.io/acme/shop@sha256:{}", "a".repeat(64));
        assert!(is_digest(&digest));
        assert!(is_digest(&format!(
            "localhost:5000/shop@sha256:{}",
            "0".repeat(64)
        )));
        assert!(!is_digest("ghcr.io/acme/shop:main"));
        assert!(!is_digest(&format!("shop@sha256:{}", "A".repeat(64))));
        assert_eq!(
            digest_reference(&format!("docker-pullable://{digest}")).as_deref(),
            Some(digest.as_str())
        );
        assert_eq!(digest_reference("sha256:abc"), None);
    }

    #[test]
    fn finds_the_repository_of_a_reference() {
        assert_eq!(repository("ghcr.io/acme/shop:main"), "ghcr.io/acme/shop");
        assert_eq!(repository("localhost:5000/shop"), "localhost:5000/shop");
        assert_eq!(repository("localhost:5000/shop:v2"), "localhost:5000/shop");
        assert_eq!(repository("shop@sha256:abc"), "shop");
        assert_eq!(normalize("docker.io/library/nginx"), "nginx");
    }
}
