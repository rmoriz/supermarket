CARGO ?= cargo
COMPOSE = docker compose -f harness/docker-compose.test.yml

.PHONY: all build test fmt clippy check up down clean

all: check

build:
	$(CARGO) build

test:
	$(CARGO) test

fmt:
	$(CARGO) fmt

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

up:
	$(COMPOSE) up -d --wait

down:
	$(COMPOSE) down -v

clean:
	$(CARGO) clean

# Full gate: formatting, lints, tests, and the corpus replay.
check: up
	$(CARGO) fmt --check
	$(CARGO) clippy --all-targets -- -D warnings
	$(CARGO) test
	harness/bin/replay harness/corpus/smoke.yml
