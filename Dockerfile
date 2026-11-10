# --- build ---
FROM rust:1-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p mygraph-server

# --- runtime ---
FROM gcr.io/distroless/cc-debian12
COPY --from=build /src/target/release/mygraph-server /usr/local/bin/mygraph-server
EXPOSE 6660
VOLUME /data
ENTRYPOINT ["mygraph-server", "start", "--host", "0.0.0.0", "--port", "6660", "--data", "/data"]
