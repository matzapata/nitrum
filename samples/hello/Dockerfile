ARG DATA_PLANE_IMAGE=nitrum/data-plane:latest
FROM --platform=linux/amd64 ${DATA_PLANE_IMAGE}

# Node 16 from NodeSource (last LTS that works on AL2 glibc 2.26; Node 18+ needs glibc 2.28)
RUN curl -fsSL https://rpm.nodesource.com/setup_16.x | bash - \
    && yum install -y nodejs \
    && yum clean all

WORKDIR /app
COPY package.json .
RUN npm install
COPY src/main.js src/main.js

COPY nitrum.toml /app/nitrum.toml

CMD ["--config", "/app/nitrum.toml", "--", "node", "/app/src/main.js"]
