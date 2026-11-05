# Security policy

Report vulnerabilities through GitHub private vulnerability reporting for this repository.
Do not open a public issue.

Include the affected command or component, the impact, and steps to reproduce. Do not
include real customer data.

The latest release receives security fixes.

Treat runtime tokens and the session of `ssp login` as secrets. `ssp` prints a secret
only once, in the response that creates it.

## Verify a release

The release workflow signs the image `ghcr.io/statespace-tech/statespace` with cosign,
without a key. It also attaches an SBOM and a build provenance to the image, and
records a build provenance for each archive of the GitHub release.

1. Verify the signature of the image. Replace `<version>` with the version, such as
   `0.5.0`.

   ```shell
   cosign verify ghcr.io/statespace-tech/statespace:<version> \
     --certificate-oidc-issuer https://token.actions.githubusercontent.com \
     --certificate-identity-regexp \
       '^https://github\.com/statespace-tech/statespace/\.github/workflows/release\.yml@refs/tags/v'
   ```

2. Verify the provenance of an archive.

   ```shell
   gh attestation verify ssp-x86_64-unknown-linux-musl.tar.gz \
     --repo statespace-tech/statespace
   ```

3. Show the SBOM of the image.

   ```shell
   docker buildx imagetools inspect ghcr.io/statespace-tech/statespace:<version> \
     --format '{{ json .SBOM }}'
   ```
