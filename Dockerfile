FROM rust:1.98-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock* rust-toolchain.toml ./
RUN mkdir src && printf 'fn main() {}' > src/main.rs && cargo build --release
COPY . .
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd --system --create-home --uid 10001 loghmeh
COPY --from=build /app/target/release/loghmeh /usr/local/bin/loghmeh
USER loghmeh
ENV RUST_LOG=info
EXPOSE 8080
CMD ["loghmeh"]
