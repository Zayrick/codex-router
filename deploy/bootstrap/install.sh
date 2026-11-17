#!/bin/bash
# Bash 3.2-compatible installer for macOS 11 and later.
main() (
    set -euo pipefail
    umask 077
    service_url='@@PUBLIC_URL@@'
    expected_hash='@@MAC_CLIENT_SHA256@@'
    work_dir=''
    files=('' '')
    stages=('' '')
    backups=('' '')
    written=(0 0)
    completed=0

    fail() { printf '%s\n' "$1" >&2; exit 1; }
    cleanup() {
        status=$?
        trap - EXIT INT TERM
        if [[ "$completed" -eq 0 ]]; then
            for i in 0 1; do
                if [[ "${written[$i]}" -eq 1 ]]; then
                    if [[ -n "${backups[$i]}" ]]; then
                        cp -p "${backups[$i]}" "${files[$i]}" || printf '%s\n' "恢复失败，请手动还原备份：${backups[$i]}" >&2
                    else
                        rm -f -- "${files[$i]}" || printf '%s\n' "清理失败，请检查文件：${files[$i]}" >&2
                    fi
                fi
            done
        fi
        for path in "${stages[@]}"; do
            if [[ -n "$path" ]]; then
                rm -f -- "$path" || printf '%s\n' "清理临时文件失败：$path" >&2
            fi
        done
        if [[ -n "$work_dir" ]]; then
            case "$work_dir" in
                "$temp_root"/codex-bootstrap.*) rm -rf -- "$work_dir" ;;
            esac
        fi
        exit "$status"
    }
    trap cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM

    [[ "$(uname -s)" == Darwin ]] || fail '此脚本适用于 macOS。'
    version=$(sw_vers -productVersion)
    [[ "${version%%.*}" -ge 11 ]] || fail '此脚本需要 macOS 11 或更高版本。'
    [[ -n "${HOME:-}" ]] || fail '无法确定当前用户的主目录，请在终端中重新运行。'
    codex_dir="$HOME/.codex"
    if [[ -n "${CODEX_HOME:-}" && "${CODEX_HOME%/}" != "$codex_dir" ]]; then
        fail 'CODEX_HOME 指向了其他目录。请先清除此环境变量，再安装到 ~/.codex。'
    fi
    for tool in curl shasum; do
        command -v "$tool" >/dev/null || fail "系统缺少 $tool，请联系管理员。"
    done
    # Read from the terminal when the script itself arrives through a pipe.
    printf '%s' '请输入你的名字：' >/dev/tty
    IFS= read -r name </dev/tty || fail '读取名字失败，请在交互式终端中运行。'

    temp_root="${TMPDIR:-/tmp}"
    temp_root="${temp_root%/}"
    work_dir=$(mktemp -d "$temp_root/codex-bootstrap.XXXXXXXX") || fail '创建临时目录失败。'
    helper="$work_dir/codex-config-edit"
    curl -fsSL "$service_url/codex-config-edit-macos" -o "$helper" || fail '下载配置助手失败，请检查网络连接后重试。'
    actual_hash=$(shasum -a 256 "$helper")
    actual_hash="${actual_hash%% *}"
    [[ "$actual_hash" == "$expected_hash" ]] || fail '下载的配置助手校验失败，请重新运行安装脚本。'
    chmod 700 "$helper"
    printf '%s' "$name" | "$helper" registration-json >"$work_dir/request.json"
    curl -fsS "$service_url/register" -H 'Content-Type: application/json; charset=utf-8' \
        --data-binary "@$work_dir/request.json" -o "$work_dir/response.json" || fail '领取 API key 失败，请检查名字和服务器状态后重试。'

    mkdir -p "$codex_dir" || fail '创建 Codex 配置目录失败。'
    auth_stage=$(mktemp "$codex_dir/.auth-XXXXXXXX")
    stages[0]="$auth_stage"
    config_stage=$(mktemp "$codex_dir/.config-XXXXXXXX")
    stages[1]="$config_stage"
    files=("$codex_dir/auth.json" "$codex_dir/config.toml")
    base_url=$("$helper" prepare-auth "$work_dir/response.json" "$auth_stage" "$work_dir/headers")
    "$helper" "${files[1]}" "$config_stage" "$base_url" || fail '准备 config.toml 失败，请根据上方提示处理后重试。原有文件未修改。'

    printf '%s\n' '正在等待代理服务器启用你的 API key……'
    ready=0
    for ((attempt=0; attempt<30; attempt++)); do
        if curl -fsS --max-time 10 "${base_url%/}/models" -H "@$work_dir/headers" -o /dev/null 2>/dev/null; then
            ready=1
            break
        fi
        sleep 0.3
    done
    [[ "$ready" -eq 1 ]] || fail '代理服务器尚未接受此 API key，请检查服务器状态后重试。'

    timestamp="$(date +%Y%m%d-%H%M%S)-$$"
    for i in 0 1; do
        backups[$i]=''
        if [[ -e "${files[$i]}" || -L "${files[$i]}" ]]; then
            [[ -f "${files[$i]}" && ! -L "${files[$i]}" ]] || fail "配置路径必须是普通文件：${files[$i]}"
            backups[$i]="${files[$i]}.bak-$timestamp"
            [[ ! -e "${backups[$i]}" ]] || fail "备份文件已存在，请重新运行脚本：${backups[$i]}"
            cp -p "${files[$i]}" "${backups[$i]}" || fail '备份原文件失败，请检查文件权限。'
            chmod 600 "${backups[$i]}"
        fi
    done
    for i in 0 1; do
        written[$i]=1
        mv -f "${stages[$i]}" "${files[$i]}" || fail '写入配置失败，正在恢复原文件。'
    done
    completed=1
    printf '已为 %s 完成 Codex 配置。\n' "$name"
    printf 'API 地址：%s\n' "$base_url"
    for path in "${backups[@]}"; do
        [[ -z "$path" ]] || printf '原文件备份：%s\n' "$path"
    done
    printf '\033[33m%s\033[0m\n' '请重启 ChatGPT 客户端 / Codex，使新配置生效。'
)

main "$@"
