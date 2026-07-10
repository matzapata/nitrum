.PHONY: lint format check docs-diagrams test

lint:
	cargo clippy --all-targets --all-features -- -D warnings

format:
	cargo fmt --all

check: lint format

docs-diagrams:
	poetry install --no-root --with diagrams
	poetry run python docs/diagrams/render_all.py

test:
	cargo test --all-targets --all-features