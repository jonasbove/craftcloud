#!/usr/bin/env bash
# CraftCloud's own checks in the Packaging lint workflow, run by craft-common-actions' packaging-lint.yml
# (`extra-checks-script`) after its shared lint, with its tools (shellcheck) installed. The shared lint covers
# packaging/; this covers what lets Claude's answers through to GitHub (claude-pr.yml, claude-issues.yml):
# the scripts in .github/scripts, the label and gitmoji lists, and claude-bot.sh fed hostile input. Offline.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

echo "==> shellcheck .github/scripts"
find .github/scripts -type f -name '*.sh' -print0 | xargs -0 --no-run-if-empty shellcheck

echo "==> labels.json and gitmojis.json"
jq -e '(map(.name) | unique | length) == length and all(.[]; (.name | type) == "string" and (.group | IN("type", "platform", "area", "state", "priority")))' .github/labels.json
jq -e '.gitmojis | length > 0' .github/gitmojis.json

echo "==> claude-bot.sh"
.github/scripts/claude-bot-test.sh
