#!/bin/sh
# mmhmem: this installer has moved.
#
# This file only prints the new install command. It downloads nothing, changes
# nothing and ignores every argument (old options such as --agent, --hooks or
# --remove are now chosen in mmhmem setup). It works with "| sh", "| bash" and
# "| bash -s -- <options>". The real installer is a per-release asset:
#   curl -fsSL https://github.com/norandom/mmhmem/releases/latest/download/install.sh | sh
#
# Exit code 2 means "nothing was installed, use the new command".
cat >&2 <<'EOF'
The mmhmem installer has moved. This script did not download or change anything.

To install mmhmem, run:

  curl -fsSL https://github.com/norandom/mmhmem/releases/latest/download/install.sh | sh

It installs the latest signed release and opens mmhmem setup, where you choose
your tools and features. Options such as --agent or --hooks are no longer used.

Updating? With mmhmem v0.5.1 or later run: mmhmem update
With v0.5.0 run: mmhmem setup. Older versions: use the command above.
EOF
exit 2
