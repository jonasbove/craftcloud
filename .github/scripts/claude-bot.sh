#!/usr/bin/env bash
# Helpers for the Claude workflows (claude-pr.yml, claude-issues.yml).
#
# Claude only reads and answers in JSON; these functions are the guard between its answer and the
# repository. Everything they get from a pull request, an issue or Claude is untrusted: it arrives
# as an argument, a file or an environment variable, is never evaluated, and only names on the
# fixed lists (.github/gitmojis.json, .github/labels.json) ever reach GitHub.
#
# Subcommands (run with no arguments for usage); claude-bot-test.sh tests them.
#   starts-with-emoji TITLE              exit 0 when TITLE starts with an emoji
#   gitmoji-title TITLE CODE             print "<gitmoji> TITLE" when it should be set, else nothing
#   filter-labels PROPOSED GROUPS EXTRA SKIP
#                                        print the JSON array of labels that may be added
#   sensitive-paths FILES_JSON           print the changed paths that need a careful look (JSON)
#   hidden-chars NAME < TEXT             print hidden or direction-changing characters (JSON lines)
#   scan-patches FILES_JSON              hidden-chars over the added lines of every patch (JSON)
#   sanitize < TEXT                      one line of text that is safe to post in a comment
#   looks-secret < TEXT                  exit 0 when TEXT (or a string in it, if JSON) looks like a credential
#   withheld-review                      print the fixed comment posted instead of a withheld review
#   save-answer EXECUTION_FILE OUT       write Claude's JSON answer to OUT, or {"withheld":true} when
#                                        it looks like a credential; prints only saved/withheld/none
#   answer-state FILE                    print ok, withheld or none for a saved answer (scans it again)
#   render-review REVIEW_JSON FACTS_JSON SHA RUN_URL
#                                        print the review comment (Markdown)
#   fetch-pr REPO NUMBER DIR             write a pull request's data into DIR (needs GH_TOKEN)
#   fetch-issue REPO NUMBER DIR          write an issue's data into DIR (needs GH_TOKEN)
#
# Needs bash, jq, perl (5.32 or newer, for \p{Extended_Pictographic}) and, for fetch-*, gh.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GITMOJIS="${GITMOJIS:-$here/../gitmojis.json}"
LABELS="${LABELS:-$here/../labels.json}"

# The marker that finds the sticky review comment again. Only a comment by github-actions[bot] that
# starts with it counts (claude-pr.yml), so a person can't hijack it by quoting it.
REVIEW_MARKER='<!-- craftcloud-claude-review -->'

# GitHub's limit on a pull request title.
TITLE_MAX=256
# At most this many labels from one run.
LABELS_MAX=8
# Caps on what Claude reads, so a huge pull request can't run the job out of time.
DIFF_CAP_BYTES=400000
COMMITS_WITH_FILES=50

die() {
  printf 'claude-bot: %s\n' "$*" >&2
  exit 2
}

# Exit 0 when $1 starts with an emoji: an Extended_Pictographic character (which covers ©, ® and ™
# as well), a flag (regional indicators) or a keycap (1️⃣, #️⃣).
starts_with_emoji() {
  T="${1-}" perl -CSDA -Mutf8 -e '
    my $t = $ENV{T}; utf8::decode($t);
    exit(($t =~ /\A(?:\p{Extended_Pictographic}|\p{Regional_Indicator}|[0-9#*]\x{FE0F}?\x{20E3})/) ? 0 : 1);'
}

# Print the title to set: the gitmoji for CODE (":bug:", looked up exactly in gitmojis.json), a
# space, then TITLE unchanged. Print nothing when TITLE already starts with an emoji, is empty, the
# code is not a gitmoji, or the result would be too long.
gitmoji_title() {
  local title="${1-}" code="${2-}" emoji new
  if [[ -z "${title//[[:space:]]/}" ]]; then
    echo "empty title, left alone" >&2
    return 0
  fi
  if starts_with_emoji "$title"; then
    echo "the title already starts with an emoji, left alone" >&2
    return 0
  fi
  emoji="$(jq -r --arg code "$code" '[.gitmojis[] | select(.code == $code) | .emoji] | first // empty' "$GITMOJIS")"
  if [[ -z "$emoji" ]]; then
    echo "not a gitmoji code: $(printf '%s' "$code" | sanitize | cut -c1-80)" >&2
    return 0
  fi
  new="$emoji $title"
  if (($(printf '%s' "$new" | perl -CSD -ne 'print length') > TITLE_MAX)); then
    echo "the title would be longer than $TITLE_MAX characters, left alone" >&2
    return 0
  fi
  printf '%s\n' "$new"
}

# PROPOSED: Claude's JSON array of label names. GROUPS: comma-separated groups from labels.json a
# bot may add (type,platform,area). EXTRA: JSON array of further names allowed this time. SKIP: JSON
# array of names never to add (already on it, or removed by a person before). Prints a JSON array:
# exact names from labels.json only, no duplicates, at most LABELS_MAX.
filter_labels() {
  local proposed="${1-[]}" groups="${2-}" extra="${3-[]}" skip="${4-[]}"
  jq -c -n \
    --argjson proposed "$(jq -c 'if type == "array" then . else [] end' <<<"$proposed" 2>/dev/null || echo '[]')" \
    --arg groups "$groups" \
    --argjson extra "$(jq -c 'if type == "array" then . else [] end' <<<"$extra" 2>/dev/null || echo '[]')" \
    --argjson skip "$(jq -c 'if type == "array" then . else [] end' <<<"$skip" 2>/dev/null || echo '[]')" \
    --slurpfile labels "$LABELS" --argjson max "$LABELS_MAX" '
    # Names are compared without U+FE0F (the emoji presentation selector, which "⌨️ cli" has and a
    # model may drop or add); what is added is always the spelling in labels.json.
    def key: gsub("\ufe0f"; "");
    ($groups | split(",") | map(select(length > 0))) as $g
    | ([$labels[0][] | select(.group as $x | $g | index($x)) | .name | key] + [$extra[] | strings | key]) as $allowed
    | ($skip | map(strings | key)) as $skipped
    | reduce ($proposed[] | strings) as $p ([];
        ([$labels[0][].name | select(key == ($p | key))] | first) as $n
        | if $n != null and ($allowed | index($n | key)) != null
             and ($skipped | index($n | key)) == null and (index($n)) == null
          then . + [$n] else . end)
    | .[:$max]'
}

# Changed paths that can run code at build, install or CI time, or steer the people and agents who
# work on the repository. FILES_JSON: the pull request's files (fetch-pr writes files.json).
sensitive_paths() {
  jq -c '[.[] | (.filename, (.previous_filename // empty)) | strings | select(
      test("(^|/)build\\.rs$") or test("(^|/)Cargo\\.(toml|lock)$") or test("^\\.cargo/")
      or test("^\\.github/") or test("^packaging/") or test("(^|/)Dockerfile[^/]*$") or test("^\\.dockerignore$")
      or test("^docker/") or test("^scripts/") or test("^xtask/") or test("^flake\\.(nix|lock)$")
      or test("^nix/") or test("^deny\\.toml$") or test("^rust-toolchain") or test("^compose\\.ya?ml$")
      or test("^\\.gitmodules$") or test("^upstream/") or test("^apps\\.toml$")
      or test("(^|/)(AGENTS|CLAUDE)(\\.local)?\\.md$") or test("^\\.claude/") or test("^\\.mcp\\.json$")
      or test("\\.(sh|ps1|bat|cmd|py|js|mjs|ts)$"))] | unique' "$1"
}

# Hidden or direction-changing characters, read from stdin; NAME says where the text came from.
# One JSON object per finding: {"where": NAME, "line": N, "char": "U+202E", "name": "..."}.
# A zero-width joiner between two pictographs (as in the technologist emoji) is part of an emoji and not reported.
hidden_chars() {
  # perl finds them (line, code point, name, tab-separated); jq turns them into JSON with NAME.
  perl -CSD -Mutf8 -Mcharnames=:full -ne '
    my $line = $.;
    while (/([\x{200B}-\x{200F}\x{202A}-\x{202E}\x{2060}-\x{2064}\x{2066}-\x{2069}\x{206A}-\x{206F}\x{061C}\x{180E}\x{FEFF}\x{115F}\x{1160}\x{3164}\x{FFA0}\x{E0000}-\x{E007F}\x{E0100}-\x{E01EF}])/g) {
      my $c = $1; my $pos = pos() - 1;
      if ($c eq "\x{200D}") {
        my $before = $pos > 0 ? substr($_, $pos - 1, 1) : "";
        my $after = substr($_, $pos + 1, 1);
        next if $before =~ /[\p{Extended_Pictographic}\x{FE0F}\x{1F3FB}-\x{1F3FF}]/ && $after =~ /\p{Extended_Pictographic}/;
      }
      my $cp = sprintf("U+%04X", ord $c);
      print join("\t", $line, $cp, charnames::viacode(ord $c) // "unnamed"), "\n";
    }' | jq -R -c --arg where "${1-text}" 'split("\t") | {where: $where, line: (.[0] | tonumber), char: .[1], name: .[2]}'
}

# hidden-chars over the added lines of every patch in FILES_JSON. Prints one JSON array; "line" is
# the line within that file's patch.
scan_patches() {
  local files="$1" out
  out="$(jq -r '.[] | [.filename, (.patch // "")] | @json' "$files" | while IFS= read -r row; do
    name="$(jq -r '.[0]' <<<"$row")"
    jq -r '.[1]' <<<"$row" | perl -CSD -pe 's/^[^+].*//; s/^\+//' | hidden_chars "$name"
  done)"
  jq -s -c '.' <<<"${out:-}"
}

# One line of text that is safe to post: no control, format, bidi or zero-width characters, no
# HTML tags (so no hidden comments or a fake review marker), no images, no @-mentions (which would
# notify people), at most 1500 characters.
sanitize() {
  perl -CSD -Mutf8 -0777 -ne '
    s/\r\n?/\n/g;
    s/[\x{0}-\x{8}\x{B}-\x{1F}\x{7F}-\x{9F}]//g;
    s/[\p{Cf}\x{115F}\x{1160}\x{3164}\x{FFA0}\x{E0100}-\x{E01EF}]//g;
    s/\s+/ /g; s/^ //; s/ $//;
    s/<(?=[A-Za-z!\/?])/&lt;/g;
    s/!\[/[/g;
    s/(?<![\w`])@([A-Za-z0-9][A-Za-z0-9-]{0,38})/`\@$1`/g;
    $_ = substr($_, 0, 1497) . "..." if length($_) > 1500;
    print;'
}

# Exit 0 when the text on stdin looks like it holds a credential. Claude's job has tokens in its
# environment; if a pull request talked it into reading one, it must not reach a public comment or
# summary. Checks the text as given and, when it is JSON, every string in it after unescaping. Never
# prints what it found. Known token shapes, a private key, a JWT, or a long run of base64-like
# characters (not counting "/", so paths and URLs split) with upper case, lower case and digits and
# a high entropy (a git SHA or a checksum, all lower-case hex, doesn't count).
looks_secret() {
  local text
  text="$(cat)"
  {
    printf '%s\n' "$text"
    jq -r '.. | strings' <<<"$text" 2>/dev/null || true
  } | perl -0777 -ne '
    my $t = $_;
    exit 0 if $t =~ /sk-ant-[A-Za-z0-9_-]{10,}/;
    exit 0 if $t =~ /gh[opsur]_[A-Za-z0-9]{20,}/;
    exit 0 if $t =~ /github_pat_[A-Za-z0-9_]{20,}/;
    exit 0 if $t =~ /xox[abpr]-[A-Za-z0-9-]/;
    exit 0 if $t =~ /AKIA[0-9A-Z]{16}/;
    exit 0 if $t =~ /-----BEGIN [A-Z ]*PRIVATE KEY-----/;
    exit 0 if $t =~ /eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\./;
    while ($t =~ /([A-Za-z0-9+=_-]{40,})/g) {
      my $r = $1;
      next unless $r =~ /[A-Z]/ && $r =~ /[a-z]/ && $r =~ /[0-9]/;
      my (%seen, $h);
      $seen{$_}++ for split //, $r;
      for (values %seen) { my $p = $_ / length($r); $h -= $p * log($p) / log(2); }
      exit 0 if $h >= 4.3;
    }
    exit 1;'
}

# Claude's answer (the structured output of the action's last successful result) from the
# action's execution file, written to OUT for the artifact the apply job downloads. The answer never
# goes through env, step outputs or the log, because GitHub prints those and anyone can read a
# public repository's run logs and, while they exist, its artifacts. So it is scanned here first,
# and one that looks like a credential is replaced by a fixed placeholder. Prints only a word.
save_answer() {
  local exec_file="$1" out="$2" answer
  answer="$(jq -c '[.[]? | objects | select(.type == "result")] | last // {}
    | select(.subtype == "success" and (.is_error | not)) | .structured_output // empty | objects' \
    "$exec_file" 2>/dev/null || true)"
  if [[ -z "$answer" ]]; then
    rm -f "$out"
    echo none
  elif looks_secret <<<"$answer"; then
    printf '{"withheld":true}\n' >"$out"
    echo withheld
  else
    printf '%s\n' "$answer" >"$out"
    echo saved
  fi
}

# The state of a saved answer, checked again after download: ok, withheld (the placeholder, or a
# file that now looks like a credential) or none (missing or not a JSON object). Prints only a word.
answer_state() {
  local file="$1"
  if [[ ! -s "$file" ]] || ! jq -e 'type == "object"' "$file" >/dev/null 2>&1; then
    echo none
  elif jq -e '.withheld == true' "$file" >/dev/null 2>&1 || looks_secret <"$file"; then
    echo withheld
  else
    echo ok
  fi
}

# The comment posted instead of a review that looked like it held a credential.
withheld_review() {
  printf '%s\n' "$REVIEW_MARKER"
  printf '## Automated review\n\n'
  printf 'The automated review was withheld; a maintainer will look at this pull request.\n'
}

# The review comment, from Claude's answer (REVIEW_JSON, claude-pr.yml's schema), the workflow's own
# findings (FACTS_JSON: {"sensitive": [...], "hidden": [...]}), the reviewed commit and the run.
render_review() {
  local review="${1-}" facts="${2-}" sha="${3-}" run_url="${4-}"
  jq -e 'type == "object"' >/dev/null 2>&1 <<<"$review" || review='{}'
  jq -e 'type == "object"' >/dev/null 2>&1 <<<"$facts" || facts='{}'
  [[ "$sha" =~ ^[0-9a-f]{7,40}$ ]] || sha=""
  [[ "$run_url" =~ ^https://github\.com/[A-Za-z0-9._/-]+$ ]] || run_url=""

  field() { jq -r --arg k "$1" '.[$k] // "" | if type == "string" then . else tostring end' <<<"$review" | sanitize; }
  list() {
    local items
    items="$(jq -r --arg k "$1" '.[$k] // [] | if type == "array" then .[:15][] else empty end | strings | @json' <<<"$review")"
    if [[ -z "$items" ]]; then
      printf '%s\n' "$2"
      return
    fi
    while IFS= read -r item; do
      printf -- '- %s\n' "$(jq -r '.' <<<"$item" | sanitize)"
    done <<<"$items"
  }
  # shellcheck disable=SC2016 # the backticks are Markdown
  code_span() { printf '`%s`' "$(sanitize | tr -d '`')"; }

  local verdict summary notes
  summary="$(field summary)"
  notes="$(field matches_description_notes)"
  case "$(jq -r '.matches_description // ""' <<<"$review")" in
    yes) verdict="Yes." ;;
    partly) verdict="Partly." ;;
    no) verdict="No." ;;
    *) verdict="Unclear." ;;
  esac

  printf '%s\n' "$REVIEW_MARKER"
  printf '## Automated review\n\n'
  printf '> This is an automated review by Claude, run by this repository'"'"'s GitHub Actions. It can be wrong, and it is not an approval: a maintainer reads every pull request and decides on merging. It is updated on every push.\n\n'
  if ! jq -e 'length > 0' >/dev/null <<<"$review"; then
    printf 'Claude did not finish this review; a maintainer will look at the pull request.\n\n'
  else
    printf '**What it does:** %s\n\n' "${summary:-(no summary)}"
    printf '**Does it do what the title and description say?** %s %s\n\n' "$verdict" "$notes"
    printf '### Must fix\n\n'
    list must_fix 'Nothing found.'
    printf '\n### Could be better\n\n'
    list could_be_better 'Nothing to add.'
    printf '\n### Risk and safety\n\n'
    list risks 'Nothing found.'
    printf '\n### House rules (AGENTS.md)\n\n'
    list house_rules 'Nothing found.'
    if [[ "$(jq -r '.instructions_to_ai // false' <<<"$review")" == "true" ]]; then
      printf '\n**Note:** the pull request contains text addressed to AI tools. It was not followed. %s\n' "$(field instructions_to_ai_notes)"
    fi
  fi

  printf '\n### Checked by the workflow\n\n'
  local paths n
  paths="$(jq -r '.sensitive // [] | if type == "array" then .[:40][] else empty end | strings | @json' <<<"$facts")"
  if [[ -n "$paths" ]]; then
    printf -- '- Changes that can run code at build, install or CI time, or steer contributors and agents; worth a careful look:'
    while IFS= read -r p; do printf ' %s' "$(jq -r '.' <<<"$p" | code_span)"; done <<<"$paths"
    printf '\n'
  else
    printf -- '- No build scripts, dependencies, CI, packaging or agent instructions changed.\n'
  fi
  # hidden_total counts them all; hidden lists the first ones.
  n="$(jq -r '(.hidden // [] | if type == "array" then length else 0 end) as $listed
    | .hidden_total // $listed | if type == "number" and . >= $listed and . < 1000000 then floor else $listed end' <<<"$facts")"
  [[ "$n" =~ ^[0-9]+$ ]] || n=0
  if ((n > 0)); then
    printf -- '- **Hidden or direction-changing characters (%s):**' "$n"
    jq -r '.hidden // [] | if type == "array" then .[:20][] else empty end | objects | [(.where // "" | tostring), (.line // 0 | tostring), (.char // "" | tostring), (.name // "" | tostring)] | @json' <<<"$facts" |
      while IFS= read -r h; do
        printf ' %s line %s %s (%s);' "$(jq -r '.[0]' <<<"$h" | code_span)" "$(jq -r '.[1]' <<<"$h" | sanitize | tr -cd '0-9')" \
          "$(jq -r '.[2]' <<<"$h" | sanitize | tr -cd 'U+0-9A-F')" "$(jq -r '.[3]' <<<"$h" | sanitize | tr -cd 'A-Z0-9 -')"
      done
    printf '\n'
  else
    printf -- '- No hidden or direction-changing characters in the added lines, the title, the description or the commit messages.\n'
  fi

  printf '\n<sub>'
  [[ -n "$sha" ]] && printf 'Reviewed commit %s. ' "${sha:0:12}"
  [[ -n "$run_url" ]] && printf '[Workflow run](%s). ' "$run_url"
  printf 'Contributors don'"'"'t need gitmoji: the workflow adds one to the title.</sub>\n'
}

# A pull request's data for Claude, read through the API (the pull request's code is never checked
# out or run): pr.json, title.txt, body.md, files.json, files.txt, diff.txt, commits.txt.
fetch_pr() {
  local repo="$1" number="$2" dir="$3" size=0 capped=0
  [[ "$number" =~ ^[0-9]+$ ]] || die "not a number: $number"
  mkdir -p "$dir"
  gh api "repos/$repo/pulls/$number" >"$dir/raw.json"
  jq '{number, author: .user.login, author_association, draft, base: .base.ref, head_sha: .head.sha,
       head_repo: .head.repo.full_name, from_fork: (.head.repo.full_name != .base.repo.full_name),
       commits, changed_files, additions, deletions}' "$dir/raw.json" >"$dir/pr.json"
  jq -r '.title // ""' "$dir/raw.json" >"$dir/title.txt"
  jq -r '.body // ""' "$dir/raw.json" >"$dir/body.md"
  rm "$dir/raw.json"

  gh api --paginate "repos/$repo/pulls/$number/files?per_page=100" --jq '.[]' | jq -s '.' >"$dir/files.json"
  jq -r '.[] | "\(.status)\t+\(.additions) -\(.deletions)\t\(.filename | @json)"
             + (if .previous_filename then "\t(was \(.previous_filename | @json))" else "" end)' \
    "$dir/files.json" >"$dir/files.txt"

  : >"$dir/diff.txt"
  while IFS= read -r row; do
    local header patch
    header="$(jq -r '"=== \(.status) \(.filename | @json) +\(.additions) -\(.deletions)"' <<<"$row")"
    patch="$(jq -r '.patch // "(no patch shown: binary, or too large for the API)"' <<<"$row")"
    if ((capped == 0 && size + ${#patch} > DIFF_CAP_BYTES)); then
      capped=1
      printf '\n=== The rest of the diff is left out (over %s bytes). Read files.txt for every changed file.\n' "$DIFF_CAP_BYTES" >>"$dir/diff.txt"
    fi
    if ((capped == 0)); then
      printf '%s\n%s\n\n' "$header" "$patch" >>"$dir/diff.txt"
      size=$((size + ${#patch}))
    fi
  done < <(jq -c '.[]' "$dir/files.json")

  : >"$dir/commits.txt"
  local i=0 sha
  while IFS= read -r row; do
    sha="$(jq -r '.sha' <<<"$row")"
    [[ "$sha" =~ ^[0-9a-f]{40}$ ]] || continue
    jq -r '"=== commit \(.sha) by \(.author.login // .commit.author.name // "unknown")\n\(.commit.message)"' <<<"$row" >>"$dir/commits.txt"
    if ((i < COMMITS_WITH_FILES)); then
      gh api "repos/$repo/commits/$sha" --jq '"Files: " + ([.files[].filename] | join(", "))' >>"$dir/commits.txt"
    fi
    printf '\n' >>"$dir/commits.txt"
    i=$((i + 1))
  done < <(gh api --paginate "repos/$repo/pulls/$number/commits?per_page=100" --jq '.[] | @json' | jq -c '.')
}

# An issue's data for Claude: issue.json, title.txt, body.md, open-issues.txt (other open issues,
# for spotting a duplicate).
fetch_issue() {
  local repo="$1" number="$2" dir="$3"
  [[ "$number" =~ ^[0-9]+$ ]] || die "not a number: $number"
  mkdir -p "$dir"
  gh api "repos/$repo/issues/$number" >"$dir/raw.json"
  jq '{number, author: .user.login, author_association, state, labels: [.labels[].name]}' "$dir/raw.json" >"$dir/issue.json"
  jq -r '.title // ""' "$dir/raw.json" >"$dir/title.txt"
  jq -r '.body // ""' "$dir/raw.json" >"$dir/body.md"
  rm "$dir/raw.json"
  gh api "repos/$repo/issues?state=open&per_page=100&sort=created&direction=desc" \
    --jq ".[] | select(.pull_request | not) | select(.number != $number) | \"#\(.number)\t\(.title)\"" >"$dir/open-issues.txt"
}

usage() {
  sed -n '2,/^$/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

cmd="${1-}"
shift || true
case "$cmd" in
  starts-with-emoji) starts_with_emoji "${1-}" ;;
  gitmoji-title) gitmoji_title "${1-}" "${2-}" ;;
  filter-labels) filter_labels "${1-[]}" "${2-}" "${3-[]}" "${4-[]}" ;;
  sensitive-paths) sensitive_paths "${1:?FILES_JSON}" ;;
  hidden-chars) hidden_chars "${1-text}" ;;
  scan-patches) scan_patches "${1:?FILES_JSON}" ;;
  sanitize) sanitize ;;
  looks-secret) looks_secret ;;
  withheld-review) withheld_review ;;
  save-answer) save_answer "${1:?EXECUTION_FILE}" "${2:?OUT}" ;;
  answer-state) answer_state "${1:?FILE}" ;;
  render-review) render_review "${1-}" "${2-}" "${3-}" "${4-}" ;;
  fetch-pr) fetch_pr "${1:?REPO}" "${2:?NUMBER}" "${3:?DIR}" ;;
  fetch-issue) fetch_issue "${1:?REPO}" "${2:?NUMBER}" "${3:?DIR}" ;;
  *) usage ;;
esac
