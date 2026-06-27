.PHONY: build run fmt test clean

FOLDER ?= data
OUTPUT ?= d1.html
MAX_SIGNALS ?= 1000000
MAX_TRANSITIONS ?= 200000000
SIGNAL_REGEX ?=
RECURSIVE ?=

BINARY := target/release/vcd_report
ARGS := $(FOLDER) -o $(OUTPUT) --max-signals-per-file $(MAX_SIGNALS) --max-transitions-per-row $(MAX_TRANSITIONS)

ifneq ($(strip $(SIGNAL_REGEX)),)
ARGS += --signal-regex "$(SIGNAL_REGEX)"
endif

ifneq ($(strip $(RECURSIVE)),)
ARGS += --recursive
endif

build:
	cargo build --release

run: build
	./$(BINARY) $(ARGS)

fmt:
	cargo fmt

test:
	cargo test

clean:
	cargo clean
