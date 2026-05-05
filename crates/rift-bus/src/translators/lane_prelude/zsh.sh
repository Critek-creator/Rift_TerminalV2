# Rift lane-classification prelude for zsh.
#
# Injected via RIFT_SHELL_PRELUDE when lanes_enabled = true.
# Uses precmd + preexec hooks — appends to existing hook arrays.
#
# Sentinel format: ESC ] 6973 ; <event> [; <key>=<value>]* BEL

_rift_osc=$'\033]6973;'
_rift_bel=$'\007'

_rift_precmd() {
    local _exit=$?
    printf '%s' "${_rift_osc}CMD_END;exit=${_exit}${_rift_bel}"
    printf '%s' "${_rift_osc}PROMPT_START${_rift_bel}"
}

_rift_precmd_end() {
    printf '%s' "${_rift_osc}PROMPT_END${_rift_bel}"
}

_rift_preexec() {
    printf '%s' "${_rift_osc}CMD_START${_rift_bel}"
}

# Append to zsh hook arrays (safe with oh-my-zsh / Starship).
autoload -Uz add-zsh-hook
add-zsh-hook precmd _rift_precmd
add-zsh-hook precmd _rift_precmd_end
add-zsh-hook preexec _rift_preexec
