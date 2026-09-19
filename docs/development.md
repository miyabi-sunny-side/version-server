# 開発とリリース

[利用者向けの導入](../README.md)

Git、Rust 1.96以上、Node.js 24とnpmが必要です。checkoutのルートで実行します。

## 検証

```sh
git clone https://github.com/miyabi-sunny-side/version-server.git
cd version-server
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
npm --prefix client ci && npm --prefix client test && npm --prefix client run check
npm --prefix client run build && npm --prefix client run lint:design
cargo run   # http://127.0.0.1:3000 (client/dist を配信)
```

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

main・PR・手動CIはfrontendとRustを検査し、debug serverの起動を確認します。
PR・手動CIはcontainer buildも検査し、imageは公開しません。

`Cargo.toml`と`Cargo.lock`のpackage versionを更新し、commitします。
同じcommitに`v<major>.<minor>.<patch>` tagを付けてpushすると、Release containerが動きます。手動 release も同じ tag ref を選ぶ。tag commit の検査に成功し、tag と両 manifest の version が一致した場合だけ、そのソースから image を build / publish する。公開先は `ghcr.io/miyabi-sunny-side/version-server:<version>` と `:latest`。公開imageは監視を有効にした起動確認も通す。GitHub Releaseには、build出力と一致を検証した`<version>@sha256:...`を記載する。

Docker は Rust 1.96.0 と cargo-chef 0.1.78 を固定し、frontend・Rust 依存・本体を別の層として構築する。tag 間の中間層は製品 image と別の `:build-cache` に registry cache (`mode=max`) として保存する。本体は実際の manifest で build するため、version だけの更新でも本体を再コンパイルする。

release profile は `opt-level=3` / `lto=false` / `codegen-units=16` / `strip=true`。軽微なサイズ・メモリ増を許容して build 待ち時間を減らす方針。
