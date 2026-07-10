"use strict";

const { NodeSDK } = require("@opentelemetry/sdk-node");
const { OTLPMetricExporter } = require("@opentelemetry/exporter-metrics-otlp-grpc");
const { PeriodicExportingMetricReader } = require("@opentelemetry/sdk-metrics");

// Nitrum injects OTEL_* when OTLP export is enabled (collector endpoint, service.name,
// nitrum.component=user-app). Outside an enclave these variables are unset and export
// is effectively disabled.
const sdk = new NodeSDK({
  metricReader: new PeriodicExportingMetricReader({
    exporter: new OTLPMetricExporter(),
    exportIntervalMillis: 15_000,
  }),
});

sdk.start();

process.on("SIGTERM", () => {
  sdk.shutdown().finally(() => process.exit(0));
});
