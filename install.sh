#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
bin_dir="${HOME}/.local/bin"
bin_path="${bin_dir}/agent"

cargo build --release --manifest-path "${project_dir}/Cargo.toml"

mkdir -p "${bin_dir}"
cat > "${bin_path}" <<EOF
#!/usr/bin/env sh
exec "${project_dir}/target/release/modular-agent" "\$@"
EOF
chmod 755 "${bin_path}"

echo "Installed: ${bin_path}"
case ":\${PATH}:" in
  *:"${bin_dir}":*) ;;
  *) echo "Add this to your shell config if needed: export PATH=\"${bin_dir}:\\\$PATH\"" ;;
esac

