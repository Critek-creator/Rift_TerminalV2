# Rift lane-classification prelude for PowerShell 7+ (pwsh).
#
# Injected into the PTY session via RIFT_SHELL_PRELUDE env var when
# RiftConfig.terminal.lanes_enabled = true. Wraps the user's existing
# prompt function to emit OSC 6973 sentinels at shell boundaries.
#
# Sentinel format: ESC ] 6973 ; <event> [; <key>=<value>]* BEL
#   PROMPT_START — shell is rendering the prompt (SYS lane)
#   PROMPT_END   — prompt done, awaiting user input
#   CMD_START    — user pressed Enter, command executing
#   CMD_END;exit=N — command finished, exit code N
#
# Design: wraps $Function:prompt so user customizations (posh-git, Starship,
# oh-my-posh) continue to work. The sentinels bracket their output.
#
# Embedded into rift-mcp via include_str! — do not move to a config directory.

$_rift_osc = "`e]6973;"
$_rift_bel = "`a"

# Save user's existing prompt (posh-git, Starship, etc.)
$_rift_original_prompt = $Function:prompt

function prompt {
    # Emit CMD_END for the PREVIOUS command (exit code from $?)
    # On first prompt after spawn, $LASTEXITCODE is null — treat as 0.
    $_exit = if ($null -eq $global:LASTEXITCODE) { 0 } else { $global:LASTEXITCODE }
    [Console]::Write("${_rift_osc}CMD_END;exit=${_exit}${_rift_bel}")

    # PROMPT_START — shell is rendering the prompt
    [Console]::Write("${_rift_osc}PROMPT_START${_rift_bel}")

    # Run the original prompt function (user customizations preserved)
    $result = if ($_rift_original_prompt) { & $_rift_original_prompt } else { "PS> " }

    # PROMPT_END — prompt done, cursor awaits user input
    [Console]::Write("${_rift_osc}PROMPT_END${_rift_bel}")

    return $result
}

# PSReadLine hooks for CMD_START — fires when user presses Enter.
# AcceptLine is the standard readline accept handler.
if (Get-Module PSReadLine -ErrorAction SilentlyContinue) {
    Set-PSReadLineKeyHandler -Key Enter -ScriptBlock {
        [Console]::Write("${script:_rift_osc}CMD_START${script:_rift_bel}")
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
    }
}
