# Test the library and build its book. See README.md.

BUILD := build

.PHONY: all test check abi-layout book book-check clean

all: check test book

# Unit tests, on the host.
test:
	cargo test

# The library builds and lints for its target.
check:
	cargo clippy --target aarch64-unknown-none-softfloat

# Regenerate abi/layout.txt from the QNX SDP's headers, the layout the
# Rust ABI types are tested against. Needs the SDP's environment (source
# qnxsdp-env.sh first).
abi-layout:
	@test -n "$(QNX_TARGET)" || { echo "QNX_TARGET is not set: source qnxsdp-env.sh from the QNX SDP 8.0 first" >&2; exit 1; }
	@mkdir -p $(BUILD)
	ntoaarch64-gcc -c abi/layout.c -o $(BUILD)/layout.o
	python3 -I abi/layout.py ntoaarch64-nm $(BUILD)/layout.o abi/layout.txt

# The book, in docs/book/build.
book: book-check
	mdbook build docs/book

book-check:
	python3 -I docs/book/check.py

clean:
	rm -rf $(BUILD) docs/book/build
