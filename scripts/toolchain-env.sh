#!/usr/bin/env bash
# Optional user-local tools; standard environment variables always take priority.
atv_data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
if [[ -z "${CARGO_HOME:-}" && -x "$atv_data_home/atv-rust/cargo/bin/cargo" ]]; then
  export CARGO_HOME="$atv_data_home/atv-rust/cargo"
  export RUSTUP_HOME="${RUSTUP_HOME:-$atv_data_home/atv-rust/rustup}"
fi
if [[ -n "${CARGO_HOME:-}" ]]; then export PATH="$CARGO_HOME/bin:$PATH"; fi
if [[ -z "${ANDROID_HOME:-}" ]]; then
  if [[ -n "${ANDROID_SDK_ROOT:-}" ]]; then
    export ANDROID_HOME="$ANDROID_SDK_ROOT"
  else
    for atv_sdk in "$HOME/Android/Sdk" "$HOME/Library/Android/sdk" "$atv_data_home/android-sdk"; do
      if [[ -d "$atv_sdk/platform-tools" ]]; then export ANDROID_HOME="$atv_sdk"; break; fi
    done
  fi
fi
if [[ -z "${JAVA_HOME:-}" ]]; then
  for atv_jdk in "$atv_data_home/atv-tools/jdk-21" "$atv_data_home/rea-tools/jdk-21"; do
    if [[ -x "$atv_jdk/bin/java" ]]; then export JAVA_HOME="$atv_jdk"; break; fi
  done
fi
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-${ATV_RUST_TOOLCHAIN:-1.99.1}}"
unset atv_sdk atv_jdk atv_data_home
