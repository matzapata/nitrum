ARG DATA_PLANE_IMAGE=matzapata/nitrum-data-plane:v0.1.0-dev
FROM ${DATA_PLANE_IMAGE}

# Base is Alpine; install Node from apk (no bash/yum).
RUN apk add --no-cache nodejs npm

WORKDIR /app
COPY package.json .
RUN npm install
COPY src/main.js src/main.js

COPY nitrum.toml /app/nitrum.toml

CMD ["/app/data-plane", "--config", "/app/nitrum.toml", "--", "node", "/app/src/main.js"]
