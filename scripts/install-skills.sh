#!/usr/bin/env bash
#
# The skills are generated into `.claude/skills`, which is where Claude
# Code looks. This copies them to the other agents that read a skills
# directory, so whichever one you use finds them.
#
#   scripts/install-skills.sh            # .codex and .agents
#   scripts/install-skills.sh --link     # symlink instead of copy
#
# Symlinks keep one copy to edit; copies survive being moved or zipped.

set -euo pipefail
cd "$(dirname "$0")/.."

[[ -d .claude/skills ]] || { echo "no .claude/skills here — was the project generated without skills?"; exit 1; }

MODE="copy"
[[ "${1:-}" == "--link" ]] && MODE="link"

for target in .codex/skills .agents/skills; do
  mkdir -p "$(dirname "$target")"
  rm -rf "$target"
  if [[ "$MODE" == "link" ]]; then
    ln -s "../.claude/skills" "$target"
  else
    cp -R .claude/skills "$target"
  fi
  echo "  $MODE → $target"
done
