# Recipes for the container built from Dockerfile.libafl, with the checkout mounted at
# /stratamoto: `just -f /ci/libafl.justfile run`.

target := env("CARGO_TARGET_DIR", "/stratamoto/target/docker")
pool := env("STRATAMOTO_POOL", "/sv2/bin/pool_sv2")
share := "/tmp/stratamoto_pool_setup_connection"
cores := "0"

# The scenario is built on its own, so that its `nyx` feature reaches nothing else.
[working-directory: '/stratamoto']
compile:
	cargo build --release -p stratamoto-scenarios --features nyx
	cargo build --release -p stratamoto-cli -p stratamoto-libafl

# libafl_nyx built QEMU-Nyx and the packer into the target directory itself.
[working-directory: '/stratamoto']
compile_nyx: compile
	rm -rf {{share}}
	{{target}}/release/stratamoto init --sharedir {{share}} --scenario {{target}}/release/pool_setup_connection --pool {{pool}} --template-provider /template-provider --nyx-dir {{target}}

corpus:
	mkdir -p /tmp/in

[working-directory: '/stratamoto']
run: compile compile_nyx corpus
	{{target}}/release/stratamoto-libafl --input /tmp/in/ --output /tmp/out/ --share {{share}} --cores {{cores}} --verbose

[working-directory: '/stratamoto']
clean:
	rm -rf /tmp/in /tmp/out {{share}} && cargo clean

[working-directory: '/stratamoto']
test: compile compile_nyx corpus
	#!/bin/bash
	timeout 60s sh -c '{{target}}/release/stratamoto-libafl --input /tmp/in/ --output /tmp/out/ --share {{share}} --cores 0 --verbose > stdout.log'
	if grep -qaE "corpus: [1-9]" stdout.log; then
		echo "Fuzzer is working"
	else
		echo "Fuzzer does not generate any testcases"
		exit 1
	fi
