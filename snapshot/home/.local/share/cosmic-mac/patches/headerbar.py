#!/usr/bin/env python3
"""macOS title bar for a COSMIC app (Terminal, Text Editor, Media Player).
Run from the app's source root. Fetches its dependencies, patches libcosmic's header
bar in Cargo's checkouts (patches/libcosmic-headerbar.py: traffic lights on the left)
and makes sure this app's build recompiles libcosmic. Exits 1 if that fails.
"""
import os, subprocess, sys

here = os.path.dirname(os.path.abspath(__file__))
env = dict(os.environ, RUSTC_BOOTSTRAP="1", CARGO_NET_GIT_FETCH_WITH_CLI="true")
if subprocess.run(["cargo", "fetch", "--locked", "-Zgit=shallow-deps"], env=env).returncode != 0:
    sys.exit(1)
if subprocess.run([sys.executable, os.path.join(here, "libcosmic-headerbar.py")]).returncode != 0:
    sys.exit(1)
# Cargo never re-checks files of git dependencies, so drop this app's cached libcosmic.
target = os.path.expanduser(f"~/.cache/cosmic-mac/target-{os.path.basename(os.getcwd())}")
subprocess.run(["cargo", "clean", "--release", "-p", "libcosmic", "--target-dir", target], env=env)
