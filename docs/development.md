# 開発とリリース

[利用者向けの導入](../README.md)

Git、Rust 1.96以上、Node.js 24とnpmが必要です。checkoutのルートで実行します。

## 検証

```sh
git clone https://github.com/miyabi-sunny-side/version-server.git
cd version-server
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
npm --prefix client ci && npm --prefix client run format:check
npm --prefix client test && npm --prefix client run check
npm --prefix client run build && npm --prefix client run lint:design
cargo build --locked && bash scripts/smoke-server.sh
docker build -t version-server:smoke . && bash scripts/smoke-container.sh version-server:smoke
cargo run   # http://127.0.0.1:3000 (client/dist を配信)
```

検査はすべてcommit前に手元で通します。GitHub Actionsはtagからのreleaseだけを行い、検査やテストは実行しません。
`smoke-server.sh`はbuild済みのdebug serverを検証用DBで起動し、health・SPA・404を確認します。
`smoke-container.sh`はimageを監視有効で起動し、healthを確認します。

## Webhookの手動確認

検証用DBで起動したローカルサーバーを対象にします。
送信側のシェルにも、サーバーに設定した`GITHUB_WEBHOOK_SECRET`を用意してください。
以下は`o/r`の架空のreleaseを登録する例です。稼働データには送らないでください。

```sh
body='{"action":"published","release":{"tag_name":"v0.1.0","published_at":"2026-09-03T00:00:00Z","assets":[]},"repository":{"full_name":"o/r"}}'
sig=$(printf '%s' "$body" | openssl dgst -sha256 -hmac "$GITHUB_WEBHOOK_SECRET" | sed 's/^.* /sha256=/')
curl -sS -X POST localhost:3000/webhook/github -H "X-Hub-Signature-256: $sig" -H 'X-GitHub-Event: release' -H 'Content-Type: application/json' -d "$body"
```

## リリース

releaseの前に上の検証を手元で通します。

`Cargo.toml`と`Cargo.lock`のpackage versionを更新し、commitします。
同じcommitに`v<major>.<minor>.<patch>` tagを付けてpushすると、Release containerが動きます。手動 release も同じ tag ref を選ぶ。tag と両 manifest の version が一致した場合だけ、そのソースから image を build / publish する。公開先は `ghcr.io/miyabi-sunny-side/version-server:<version>` と `:latest`。GitHub Releaseには、build出力と一致を検証した`<version>@sha256:...`を記載する。

Docker は Rust 1.96.0 と cargo-chef 0.1.78 を固定し、frontend・Rust 依存・本体を別の層として構築する。tag 間の中間層は製品 image と別の `:build-cache` に registry cache (`mode=max`) として保存する。本体は実際の manifest で build するため、version だけの更新でも本体を再コンパイルする。

release profile は `opt-level=3` / `lto=false` / `codegen-units=16` / `strip=true`。軽微なサイズ・メモリ増を許容して build 待ち時間を減らす方針。
