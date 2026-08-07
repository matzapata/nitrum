.PHONY: lint format check deny test docs-diagrams test-node check-node

lint:
	cargo clippy --all-targets --all-features -- -D warnings

format:
	cargo fmt --all

deny:
	cargo deny check

check: lint format

test:
	cargo test --all-targets --all-features

docs-diagrams:
	poetry install --no-root --with diagrams
	poetry run python docs/diagrams/render_all.py

test-node:
	npm run build -w nitrum-node
	npm test -w nitrum-node

check-node: test-node
