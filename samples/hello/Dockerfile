ARG DATA_PLANE_IMAGE=nitrum/data-plane:dev
FROM ${DATA_PLANE_IMAGE}

RUN apt-get update && apt-get install -y --no-install-recommends \
    nodejs \
    npm \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY package.json .
RUN npm install
COPY src/main.js src/main.js

COPY nitrum.toml /app/nitrum.toml

CMD ["node", "src/main.js"]
