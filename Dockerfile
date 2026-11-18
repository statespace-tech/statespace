# The ssp image, for `ssp proxy` in a container on every runtime. The release
# workflow builds it from the musl binaries in image/<arch>/ssp.
#
# The image has no shell. It holds the static binary, CA certificates, and the user
# nonroot (65532). `statespace` is a hard link to `ssp`. The first stage makes the link
# on the build platform, so the build runs no emulated command.
FROM --platform=$BUILDPLATFORM busybox:1.38-musl@sha256:80e38f9a1b39188b8bdb28879178edb322a4402b081a9aedda1924609decd6f6 AS bin
ARG TARGETARCH
COPY image/${TARGETARCH}/ssp /out/ssp
RUN chmod 0755 /out/ssp && ln /out/ssp /out/statespace

FROM gcr.io/distroless/static-debian12:nonroot@sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab
COPY --from=bin /out/ /usr/local/bin/
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/ssp"]
