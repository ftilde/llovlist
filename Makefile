-include .env
export

ADDR ?= 0.0.0.0:8137
LISTS ?= lists
PACKAGE := com.ftilde.kauf
ACTIVITY := $(PACKAGE)/dev.dioxus.main.MainActivity
APK := target/dx/1kauf/release/android/app/app/build/outputs/apk/debug/app-debug.apk

# `path:` because a flake ref on a git repo only sees committed files, and it must point
# at nix/ alone: `path:` copies its target into the store unfiltered, target/ included.
NIX := nix --extra-experimental-features 'nix-command flakes' develop 'path:$(CURDIR)/nix' -c

.PHONY: help token server desktop apk phone check clean-lists

help:
	@echo "make token     print a fresh token and its hash for .env"
	@echo "make server    run the sync server on $(ADDR)"
	@echo "make desktop   run the desktop app"
	@echo "make apk       build the Android release APK (arm64)"
	@echo "make phone     build, install and launch on the attached phone"
	@echo "make check     fmt, clippy and tests"

token:
	@cargo run -q -p kauf-server --release -- gen-token
	@echo
	@echo "put the hash in .env as KAUF_TOKEN_HASH, give the token to each client"

server: require-token
	cargo run -p kauf-server --release -- serve --addr $(ADDR) --state-dir $(LISTS)

desktop:
	cargo run -p kauf-app --features desktop --release

apk:
	cd app && $(NIX) dx build --platform android --release --target aarch64-linux-android

phone: apk
	adb install -r $(APK)
	adb reverse tcp:$(word 2,$(subst :, ,$(ADDR))) tcp:$(word 2,$(subst :, ,$(ADDR)))
	adb shell am start -n $(ACTIVITY)
	@echo "server field: http://127.0.0.1:$(word 2,$(subst :, ,$(ADDR)))"

check:
	cargo fmt --check
	cargo clippy --all-targets
	cargo test

require-token:
	@test -n "$(KAUF_TOKEN_HASH)" || { echo "KAUF_TOKEN_HASH is not set; run 'make token' and put the hash in .env"; exit 1; }

clean-lists:
	rm -rf $(LISTS)
