lint:
	cargo fmt --all -- --check
	cargo clippy -q --all-targets -- -D warnings
	ruff check -q .
