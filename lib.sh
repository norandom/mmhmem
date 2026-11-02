#!/usr/bin/env bash
# lib.sh — shared helpers for build.sh and install.sh.
# Source this file; do not execute it directly.
#
# State knobs (set in the caller before invoking helpers):
#   LIB_DRY_RUN=1   print actions, don't run them
#   LIB_FORCE=1     replace existing symlinks/files when linking

LIB_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB_DRY_RUN="${LIB_DRY_RUN:-0}"
LIB_FORCE="${LIB_FORCE:-0}"
LIB_SKILLS_ROOT="$LIB_ROOT/deploy/skills"
LIB_MARKETPLACE_ROOT="$LIB_ROOT/.marketplace"
LIB_MARKETPLACE_CONFIG="$LIB_MARKETPLACE_ROOT/skills.json"
LIB_MARKETPLACE_TEMPLATES="$LIB_MARKETPLACE_ROOT/templates"

lib_require() {
  command -v "$1" >/dev/null || { echo "missing dependency: $1" >&2; exit 1; }
}

lib_run() {
  if [ "$LIB_DRY_RUN" -eq 1 ]; then
    printf 'DRY: %s\n' "$*"
  else
    "$@"
  fi
}

# Skills: deploy/skills dirs containing a SKILL.md.
# If LIB_SKILLS is set (space/newline-separated names), only those are emitted,
# letting a caller install/uninstall a chosen subset instead of everything.
lib_list_skills() {
  local d name
  for d in "$LIB_SKILLS_ROOT"/*/; do
    [ -f "${d}SKILL.md" ] || continue
    name="$(basename "$d")"
    if [ -n "${LIB_SKILLS:-}" ]; then
      case " $LIB_SKILLS " in *" $name "*) ;; *) continue ;; esac
    fi
    printf '%s\n' "$name"
  done
}

# Skill description char-limit (Claude Desktop / Claude Code spec).
LIB_DESC_MAX="${LIB_DESC_MAX:-1024}"

# Extract the YAML `description:` value from a SKILL.md and print its length.
# Handles plain, quoted, and `>` / `|` folded-block scalar forms.
lib_skill_desc_len() {
  local file="$1"
  awk '
    /^description:[[:space:]]*/ {
      sub(/^description:[[:space:]]*/, "")
      sub(/^[>|][-+]?[[:space:]]*/, "")
      gsub(/^"|"$/, "")
      gsub(/^'\''|'\''$/, "")
      buf = $0
      in_block = (buf == "")
      next
    }
    in_block && /^---[[:space:]]*$/ { exit }
    in_block && /^[^[:space:]]/      { exit }
    in_block {
      sub(/^[[:space:]]+/, "")
      buf = (buf == "" ? $0 : buf " " $0)
    }
    END { printf "%d", length(buf) }
  ' "$file"
}

# Validate every skill's description against LIB_DESC_MAX.
# Prints one WARN line per oversize skill and returns 1 if any were found.
lib_validate_skills() {
  local name file len bad=0
  while read -r name; do
    [ -n "$name" ] || continue
    file="$LIB_SKILLS_ROOT/$name/SKILL.md"
    len="$(lib_skill_desc_len "$file")"
    if [ "$len" -gt "$LIB_DESC_MAX" ]; then
      printf 'WARN   %-24s  description %s chars > %s — Claude Desktop / Code will silently drop this skill\n' \
        "$name" "$len" "$LIB_DESC_MAX" >&2
      bad=1
    fi
  done < <(lib_list_skills)
  return "$bad"
}

# Read one scalar from SKILL.md frontmatter. Folded/block values are returned
# as one line so they can be safely passed to the JSON template renderer.
lib_skill_field() {
  local file="$1" key="$2"
  awk -v key="$key" '
    $0 ~ ("^" key ":[[:space:]]*") {
      sub("^" key ":[[:space:]]*", "")
      sub(/^[>|][-+]?[[:space:]]*/, "")
      value = $0
      in_block = (value == "")
      if (!in_block) {
        sub(/^"/, "", value); sub(/"[[:space:]]*$/, "", value)
        sub(/^\047/, "", value); sub(/\047[[:space:]]*$/, "", value)
        print value
        exit
      }
      next
    }
    in_block && /^---[[:space:]]*$/ { print value; in_block = 0; exit }
    in_block && /^[^[:space:]]/      { print value; in_block = 0; exit }
    in_block {
      sub(/^[[:space:]]+/, "")
      value = (value == "" ? $0 : value " " $0)
    }
    END { if (in_block) print value }
  ' "$file"
}

lib__json_string() {
  jq -Rn --arg value "$1" '$value'
}

# Render a shared template. Arguments after the output path are KEY VALUE
# pairs; template tokens use {{KEY}}. jq performs literal replacement so JSON
# strings containing quotes, ampersands, or backslashes remain intact.
lib__render_template() {
  local template="$1" output="$2"; shift 2
  [ $(( $# % 2 )) -eq 0 ] || { echo "template substitutions must be KEY VALUE pairs" >&2; return 2; }
  local text key value token
  text="$(cat "$template")"
  while [ $# -gt 0 ]; do
    key="$1"; value="$2"; shift 2
    token="{{$key}}"
    text="$(printf '%s' "$text" | jq -Rrs --arg token "$token" --arg value "$value" 'split($token) | join($value)')"
  done
  if printf '%s' "$text" | grep -q '{{[A-Z_][A-Z_]*}}'; then
    echo "unresolved token in template: $template" >&2
    return 1
  fi
  mkdir -p "$(dirname "$output")"
  printf '%s\n' "$text" > "$output"
}

lib__validate_marketplace_config() {
  [ -f "$LIB_MARKETPLACE_CONFIG" ] || { echo "missing marketplace config: $LIB_MARKETPLACE_CONFIG" >&2; return 1; }
  jq -e '
    type == "object" and
    (.plugins | type == "array" and length > 0) and
    all(.plugins[];
      type == "object" and
      (.name | type == "string" and test("\\S") and length <= 64 and test("^(?!.*(?:--|\\.\\.))[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$")) and
      (.displayName | type == "string" and test("\\S")) and
      (.shortDescription | type == "string" and test("\\S")) and
      (.category | type == "string" and test("\\S")) and
      (.defaultPrompt | type == "string" and test("\\S")) and
      (.keywords | type == "array" and length > 0 and all(.[]; type == "string" and test("\\S")))
    )
  ' "$LIB_MARKETPLACE_CONFIG" >/dev/null || {
    echo "marketplace config must contain a non-empty plugins array with valid names and typed, non-empty displayName, shortDescription, category, keywords, and defaultPrompt metadata" >&2
    return 1
  }
  jq -e '[.plugins[].name] | length == (unique | length)' "$LIB_MARKETPLACE_CONFIG" >/dev/null || {
    echo "marketplace config contains duplicate plugin names" >&2
    return 1
  }

  local configured_names skill_names d
  configured_names="$(jq -r '.plugins[].name' "$LIB_MARKETPLACE_CONFIG" | LC_ALL=C sort)"
  skill_names="$({
    for d in "$LIB_SKILLS_ROOT"/*/; do
      [ -f "${d}SKILL.md" ] || continue
      basename "$d"
    done
  } | LC_ALL=C sort)"
  if [ "$configured_names" != "$skill_names" ]; then
    echo "marketplace config names must exactly match deploy/skills directories containing SKILL.md" >&2
    diff -u <(printf '%s\n' "$skill_names") <(printf '%s\n' "$configured_names") >&2 || true
    return 1
  fi
}

lib__marketplace_plugins() {
  jq -c '.plugins[]' "$LIB_MARKETPLACE_CONFIG"
}

lib__generate_plugin_package() (
  set -euo pipefail
  local plugin="$1" name display short category prompt keywords src skill_file version description manifest_name
  local plugins_root plugin_root stage_root backup_root skill_dst old_moved=0

  name="$(jq -r '.name' <<<"$plugin")"
  display="$(jq -r '.displayName' <<<"$plugin")"
  short="$(jq -r '.shortDescription' <<<"$plugin")"
  category="$(jq -r '.category' <<<"$plugin")"
  prompt="$(jq -r '.defaultPrompt' <<<"$plugin")"
  keywords="$(jq -c '.keywords' <<<"$plugin")"
  src="$LIB_SKILLS_ROOT/$name"
  skill_file="$src/SKILL.md"
  manifest_name="$(lib_skill_field "$skill_file" name)"
  version="$(jq -r '.version' "$LIB_MARKETPLACE_CONFIG")"
  description="$(lib_skill_field "$skill_file" description)"
  printf '%s' "$name" | grep -Eq '^[a-z0-9]+(-[a-z0-9]+)*$' || { echo "invalid Agent Skills name: $name" >&2; return 1; }
  [ -n "$description" ] && [ "${#description}" -le 1024 ] || { echo "invalid skill description: $name" >&2; return 1; }
  awk '
    NR == 1 { if ($0 != "---") exit 1; next }
    /^---$/ { closed=1; exit }
    /^[A-Za-z][A-Za-z-]*:/ {
      split($0, a, ":")
      if (a[1] !~ /^(name|description|license|compatibility|metadata|allowed-tools)$/) exit 1
    }
    END { if (!closed) exit 1 }
  ' "$skill_file" || { echo "invalid skill frontmatter: $name" >&2; return 1; }
  [ "$manifest_name" = "$name" ] || { echo "$name folder/frontmatter name mismatch: $manifest_name" >&2; return 1; }
  printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$' || {
    echo "$name has invalid semver: $version" >&2; return 1;
  }

  plugins_root="$LIB_ROOT/plugins"
  mkdir -p "$plugins_root"
  plugin_root="$plugins_root/$name"
  stage_root="$plugins_root/.$name.stage.$$.$RANDOM"
  backup_root="$plugins_root/.$name.backup.$$.$RANDOM"
  case "$plugin_root" in "$plugins_root/$name") ;; *) echo "unsafe plugin path: $plugin_root" >&2; return 1 ;; esac
  case "$stage_root" in "$plugins_root/.$name.stage."*) ;; *) echo "unsafe plugin stage path: $stage_root" >&2; return 1 ;; esac
  case "$backup_root" in "$plugins_root/.$name.backup."*) ;; *) echo "unsafe plugin backup path: $backup_root" >&2; return 1 ;; esac

  cleanup_plugin_stage() {
    local status=$?
    trap - EXIT
    [ ! -e "$stage_root" ] || rm -rf -- "$stage_root" || status=1
    if [ "$status" -ne 0 ] && [ "$old_moved" -eq 1 ] && [ -e "$backup_root" ]; then
      [ ! -e "$plugin_root" ] || rm -rf -- "$plugin_root" || status=1
      mv -- "$backup_root" "$plugin_root" || status=1
    fi
    exit "$status"
  }
  trap cleanup_plugin_stage EXIT

  skill_dst="$stage_root/skills/$name"
  mkdir -p "$(dirname "$skill_dst")"
  cp -R "$src" "$skill_dst"
  rm -f "$skill_dst/$name.skill" "$skill_dst/.norandom-skills-managed.json"
  find "$skill_dst" -name .DS_Store -type f -delete

  lib__render_template "$LIB_MARKETPLACE_TEMPLATES/portable-plugin.json.tpl" \
    "$stage_root/plugin.json" \
    NAME_JSON "$(lib__json_string "$name")" \
    VERSION_JSON "$(lib__json_string "$version")" \
    DESCRIPTION_JSON "$(lib__json_string "$description")" \
    KEYWORDS_JSON "$keywords" \
    SOURCE_PATH "$name"

  lib__render_template "$LIB_MARKETPLACE_TEMPLATES/codex-plugin.json.tpl" \
    "$stage_root/.codex-plugin/plugin.json" \
    NAME_JSON "$(lib__json_string "$name")" \
    VERSION_JSON "$(lib__json_string "$version")" \
    DESCRIPTION_JSON "$(lib__json_string "$description")" \
    DISPLAY_NAME_JSON "$(lib__json_string "$display")" \
    SHORT_DESCRIPTION_JSON "$(lib__json_string "$short")" \
    CATEGORY_JSON "$(lib__json_string "$category")" \
    KEYWORDS_JSON "$keywords" \
    DEFAULT_PROMPT_JSON "$(lib__json_string "$prompt")" \
    SOURCE_PATH "$name"

  lib__render_template "$LIB_MARKETPLACE_TEMPLATES/claude-plugin.json.tpl" \
    "$stage_root/.claude-plugin/plugin.json" \
    NAME_JSON "$(lib__json_string "$name")" \
    VERSION_JSON "$(lib__json_string "$version")" \
    DESCRIPTION_JSON "$(lib__json_string "$description")" \
    SOURCE_PATH "$name"

  lib__render_template "$LIB_MARKETPLACE_TEMPLATES/gemini-plugin.json.tpl" \
    "$stage_root/.gemini-plugin/plugin.json" \
    NAME_JSON "$(lib__json_string "$name")" \
    VERSION_JSON "$(lib__json_string "$version")" \
    DESCRIPTION_JSON "$(lib__json_string "$description")" \
    KEYWORDS_JSON "$keywords" \
    SOURCE_PATH "$name"

  jq empty "$stage_root/plugin.json"
  jq empty "$stage_root/.codex-plugin/plugin.json"
  jq empty "$stage_root/.claude-plugin/plugin.json"
  jq empty "$stage_root/.gemini-plugin/plugin.json"

  if [ -e "$plugin_root" ]; then
    mv -- "$plugin_root" "$backup_root"
    old_moved=1
  fi
  mv -- "$stage_root" "$plugin_root"
  if [ "$old_moved" -eq 1 ]; then rm -rf -- "$backup_root"; fi
  trap - EXIT
  printf '  plugin %-24s -> plugins/%s\n' "$name" "$name"
)

# Generate installable plugin packages from the canonical deploy/skills skill
# directories. Generated packages are committed so release tags contain the
# exact subdirectories referenced by both marketplaces.
lib_generate_plugin_packages() {
  lib_require jq
  lib__validate_marketplace_config
  local plugin
  while IFS= read -r plugin; do
    [ -n "$plugin" ] || continue
    lib__generate_plugin_package "$plugin" || return 1
  done < <(lib__marketplace_plugins)
}

lib__validate_marketplace_ref() {
  [ -n "$1" ] || { echo "marketplace ref cannot be empty" >&2; return 1; }
  case "$1" in *[[:space:]]*) echo "marketplace ref cannot contain whitespace: $1" >&2; return 1 ;; esac
}

# Generate the three catalogs from the same plugin allowlist and release ref.
lib_generate_marketplace_catalogs() {
  lib_require jq
  lib__validate_marketplace_config
  local ref="$1" plugin name display short category keywords entry codex_entries='' claude_entries='' gemini_entries='' sep=''
  local tmp_dir codex_entry_file claude_entry_file gemini_entry_file
  lib__validate_marketplace_ref "$ref"
  mkdir -p "$LIB_ROOT/tmp"
  tmp_dir="$(mktemp -d "$LIB_ROOT/tmp/marketplace.XXXXXX")"
  codex_entry_file="$tmp_dir/codex.json"
  claude_entry_file="$tmp_dir/claude.json"
  gemini_entry_file="$tmp_dir/gemini.json"

  while IFS= read -r plugin; do
    [ -n "$plugin" ] || continue
    name="$(jq -r '.name' <<<"$plugin")"
    display="$(jq -r '.displayName' <<<"$plugin")"
    short="$(jq -r '.shortDescription' <<<"$plugin")"
    category="$(jq -r '.category' <<<"$plugin")"
    keywords="$(jq -c '.keywords // []' <<<"$plugin")"

    lib__render_template "$LIB_MARKETPLACE_TEMPLATES/codex-marketplace-entry.json.tpl" \
      "$codex_entry_file" \
      NAME_JSON "$(lib__json_string "$name")" \
      CATEGORY_JSON "$(lib__json_string "$category")" \
      REF_JSON "$(lib__json_string "$ref")" \
      SOURCE_PATH "$name"
    entry="$(sed 's/^/    /' "$codex_entry_file")"
    codex_entries="${codex_entries}${sep}${entry}"

    lib__render_template "$LIB_MARKETPLACE_TEMPLATES/claude-marketplace-entry.json.tpl" \
      "$claude_entry_file" \
      NAME_JSON "$(lib__json_string "$name")" \
      DISPLAY_NAME_JSON "$(lib__json_string "$display")" \
      SHORT_DESCRIPTION_JSON "$(lib__json_string "$short")" \
      CATEGORY_JSON "$(lib__json_string "$category")" \
      KEYWORDS_JSON "$keywords" \
      REF_JSON "$(lib__json_string "$ref")" \
      SOURCE_PATH "$name"
    entry="$(sed 's/^/    /' "$claude_entry_file")"
    claude_entries="${claude_entries}${sep}${entry}"

    lib__render_template "$LIB_MARKETPLACE_TEMPLATES/gemini-marketplace-entry.json.tpl" \
      "$gemini_entry_file" \
      NAME_JSON "$(lib__json_string "$name")" \
      CATEGORY_JSON "$(lib__json_string "$category")" \
      KEYWORDS_JSON "$keywords" \
      REF_JSON "$(lib__json_string "$ref")" \
      SOURCE_PATH "$name"
    entry="$(sed 's/^/    /' "$gemini_entry_file")"
    gemini_entries="${gemini_entries}${sep}${entry}"
    sep=$',\n'
  done < <(lib__marketplace_plugins)

  lib__render_template "$LIB_MARKETPLACE_TEMPLATES/codex-marketplace.json.tpl" \
    "$LIB_ROOT/.agents/plugins/marketplace.json" PLUGIN_ENTRIES "$codex_entries"
  lib__render_template "$LIB_MARKETPLACE_TEMPLATES/claude-marketplace.json.tpl" \
    "$LIB_ROOT/.claude-plugin/marketplace.json" PLUGIN_ENTRIES "$claude_entries"
  lib__render_template "$LIB_MARKETPLACE_TEMPLATES/gemini-marketplace.json.tpl" \
    "$LIB_ROOT/.gemini-plugin/marketplace.json" PLUGIN_ENTRIES "$gemini_entries"
  jq empty "$LIB_ROOT/.agents/plugins/marketplace.json"
  jq empty "$LIB_ROOT/.claude-plugin/marketplace.json"
  jq empty "$LIB_ROOT/.gemini-plugin/marketplace.json"
  rm -rf "$tmp_dir"
  printf '  catalogs -> %s (%s)\n' "$ref" 'Codex, Claude Code, Gemini'
}

lib_generate_marketplace() {
  lib_generate_plugin_packages
  lib_generate_marketplace_catalogs "$1"
}

# Post-release operation: refuse to advertise a tag unless every configured
# plugin package and vendor manifests are actually present in that tag.
lib_update_marketplace_release() {
  local ref="$1" plugin name
  lib_require jq
  lib__validate_marketplace_config
  lib__validate_marketplace_ref "$ref"
  git -C "$LIB_ROOT" rev-parse --verify --quiet "$ref^{commit}" >/dev/null || {
    echo "unknown release ref: $ref" >&2; return 1;
  }
  while IFS= read -r plugin; do
    name="$(jq -r '.name' <<<"$plugin")"
    git -C "$LIB_ROOT" cat-file -e "$ref:plugins/$name/.codex-plugin/plugin.json" || {
      echo "$ref does not contain the Codex manifest for $name" >&2; return 1;
    }
    git -C "$LIB_ROOT" cat-file -e "$ref:plugins/$name/.claude-plugin/plugin.json" || {
      echo "$ref does not contain the Claude manifest for $name" >&2; return 1;
    }
    git -C "$LIB_ROOT" cat-file -e "$ref:plugins/$name/.gemini-plugin/plugin.json" || {
      echo "$ref does not contain the Gemini manifest for $name" >&2; return 1;
    }
    git -C "$LIB_ROOT" cat-file -e "$ref:plugins/$name/plugin.json" || {
      echo "$ref does not contain the root plugin.json for $name" >&2; return 1;
    }
  done < <(lib__marketplace_plugins)
  lib_generate_marketplace_catalogs "$ref"
}
