.PHONY: run migrate check-db seed fmt lint test test-db check up down

run:
	cargo run --locked -- serve
migrate:
	cargo run --locked -- migrate
check-db:
	cargo run --locked -- check-db
seed:
	cargo run --locked -- seed
fmt:
	cargo fmt --all
lint:
	cargo clippy --locked --all-targets -- -D warnings
test:
	cargo test --locked
test-db:
	cargo test --locked --test api_database -- --ignored
check:
	cargo fmt --all -- --check
	cargo clippy --locked --all-targets -- -D warnings
	cargo test --locked
up:
	docker compose up --build -d
down:
	docker compose down
