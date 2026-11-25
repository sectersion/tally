# Build: wasm from source (server/wasm is gitignored), then ship the bun runtime.
FROM oven/bun:1.4.5 AS build
RUN apt-get update && apt-get install -y --no-install-recommends curl build-essential && rm -rf /var/lib/apt/lists/* \
  && curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"
RUN rustup target add wasm32-unknown-unknown
WORKDIR /repo
COPY . .
WORKDIR /repo/server
RUN bun install --frozen-lockfile && bun run wasm

FROM oven/bun:1.4.5
WORKDIR /app
COPY --from=build /repo/server/node_modules ./node_modules
COPY --from=build /repo/server/wasm ./wasm
COPY --from=build /repo/server/src ./src
COPY --from=build /repo/server/static ./static
COPY --from=build /repo/server/migrations ./migrations
COPY --from=build /repo/server/package.json ./
COPY --from=build /repo/server/tsconfig.json ./
ENV NODE_ENV=production
EXPOSE 8080
CMD ["bun", "src/main.ts"]
