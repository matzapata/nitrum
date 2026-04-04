
# Data plane is built on top of alpine:3.20
ARG DATA_PLANE_IMAGE=matzapata/nitrum-data-plane:latest-dev
FROM ${DATA_PLANE_IMAGE}

# Install Node and npm
RUN apk add --no-cache nodejs npm

# Set the working directory
WORKDIR /app

# Install dependencies
COPY package.json .
RUN npm install

# Copy the enclave code
COPY src/main.js src/main.js

# Copy the nitrum configuration
COPY nitrum.toml /app/nitrum.toml

# Start the enclave
CMD ["/app/data-plane", "--config", "/app/nitrum.toml", "--", "node", "/app/src/main.js"]
