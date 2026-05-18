FROM rust:1-bookworm AS builder

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY static ./static
RUN cargo build --release

FROM debian:bookworm-slim

RUN useradd --system --create-home --home-dir /app pinginfo \
    && mkdir -p /app/data

WORKDIR /app
COPY --from=builder /src/target/release/pinginfo /app/pinginfo
COPY static /app/static
RUN chown -R pinginfo:pinginfo /app

USER pinginfo
ENV PINGINFO_BIND=0.0.0.0:8080
ENV PINGINFO_DB=/app/data/pinginfo.db
ENV PINGINFO_RETENTION_DAYS=30
EXPOSE 8080

CMD ["/app/pinginfo"]
