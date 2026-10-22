# Runtime image for the match server and matchmaker. It packages prebuilt Linux binaries
# rather than compiling, so the image ships exactly what the release attests.
# Expected layout (the release workflow stages this): dist/<amd64|arm64>/evoclash-{server,matchmaker}
FROM gcr.io/distroless/cc-debian12:nonroot

ARG TARGETARCH
COPY dist/${TARGETARCH}/evoclash-server dist/${TARGETARCH}/evoclash-matchmaker /app/

WORKDIR /app
# The matchmaker spawns evoclash-server from its own directory. For real deployments pass
# --public-host <reachable IP> so issued connect tokens point at this host, not 127.0.0.1.
EXPOSE 8080/tcp 7000-7099/udp
ENTRYPOINT ["/app/evoclash-matchmaker"]
CMD ["--listen", "0.0.0.0:8080", "--log-dir", "/tmp/logs"]
