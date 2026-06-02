# Rift lane-classification prelude for bash.
#
# Injected via RIFT_SHELL_PRELUDE when lanes_enabled = true.
# APPENDS to existing PROMPT_COMMAND — preserves user dotfile additions.
#
# Sentinel format: ESC ] 6973 ; <event> [; <key>=<value>]* BEL

_rift_osc=$'\033]6973;'
_rift_bel=$'\007'

_rift_cmd_start() {
    # Fires ONCE before the first command in a prompt cycle (via DEBUG trap).
    # Guard: only fire between PROMPT_END and the next PROMPT_START.
    if [[ "$_rift_awaiting_cmd" == "1" ]]; then
        printf '%s' "${_rift_osc}CMD_START${_rift_bel}"
        _rift_awaiting_cmd=0
    fi
}

_rift_prompt() {
    local _exit=$?
    printf '%s' "${_rift_osc}CMD_END;exit=${_exit}${_rift_bel}"
    printf '%s' "${_rift_osc}PROMPT_START${_rift_bel}"
    # CWD — current dir at prompt time (Stage 2b live cwd). $PWD is unix-style
    # on Windows git-bash; restore canonicalizes best-effort, else falls back.
    printf '%s' "${_rift_osc}CWD;${PWD}${_rift_bel}"
}

_rift_prompt_end() {
    printf '%s' "${_rift_osc}PROMPT_END${_rift_bel}"
    _rift_awaiting_cmd=1
}

# Append to PROMPT_COMMAND (semicolon-joined, preserves existing).
PROMPT_COMMAND="_rift_prompt;${PROMPT_COMMAND:+$PROMPT_COMMAND;}_rift_prompt_end"

# DEBUG trap fires before each simple command — used for CMD_START.
trap '_rift_cmd_start' DEBUG
