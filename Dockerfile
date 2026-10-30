# The ssp image, for `ssp proxy cloudrun` and `ssp proxy kubernetes`. The release
# workflow builds it from the musl binaries in image/<arch>/ssp.
FROM alpine:3.22
ARG TARGETARCH
RUN apk add --no-cache ca-certificates
COPY image/${TARGETARCH}/ssp /usr/local/bin/ssp
RUN ln -s ssp /usr/local/bin/statespace
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/ssp"]
