.PHONY: lint format check deny test docs-diagrams lint-node typecheck-node test-node check-node

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

lint-node:
	npm run lint -w nitrum-node

typecheck-node:
	npm run typecheck -w nitrum-node

test-node:
	npm test -w nitrum-node

check-node: lint-node typecheck-node test-node
