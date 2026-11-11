# Build the book: `make html`, `make serve` or `make check` (see README.md).

.PHONY: all html serve check clean

all: check html

html:
	mdbook build

serve:
	mdbook serve --open

check:
	python3 -I check.py

clean:
	rm -rf build
