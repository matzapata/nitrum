TAG_PREFIX ?= matzapata/
TAG ?= v0.1.0

.PHONY: lint format docs-diagrams

# ── dev tools ─────────────────────────────────────────

lint:
	cargo clippy --all-targets --all-features -- -D warnings

format:
	cargo fmt --all

docs-diagrams:
	poetry install --no-root --with diagrams
	poetry run python docs/diagrams/render_all.py
