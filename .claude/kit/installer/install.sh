#!/usr/bin/env bash
# One-command install: ./install.sh /path/to/your/repo
set -e
command -v python3 >/dev/null 2>&1 || { echo "python3 is required (macOS: brew install python3; Ubuntu: sudo apt-get install -y python3)"; exit 1; }
exec python3 "$(dirname "$0")/install.py" "$@"
