# version-server

家庭内 product の「いま公開されている最新 version」を 1 か所で監視し、LAN 内へ配る。外 (GitHub) を見るのはこの server だけで、worker 機は秘密を持たずにここへ問い合わせる。

- 入力 A: GitHub の `release` webhook (`POST /webhook/github`、`X-Hub-Signature-256` を検証)
- 入力 B: GitHub Releases API の polling (保険。`WATCH_REPOS` の repo を `POLL_SECS` ごと、ETag で差分だけ)
- 状態: repo ごとの最新 release (`releases`) と、tag が変わった記録 (`events`、追記のみ)。両入力は同じ `Store::ingest` を通り、新しい tag だけが event になる。古い `published_at` の release が遅れて届いても「最新」は巻き戻らない
- 出力: `GET /v1/versions`、`GET /v1/versions/{org}/{repo}`、`GET /v1/events?since=<id>&limit=<n>`、`GET /v1/events/stream?since=<id>` (SSE)。LAN 内なので認証は無く、短い間隔の polling や張りっぱなしの接続を許す
- UI: repo ごとの最新 tag と受信時刻の一覧 1 画面 (`/`)

## API

| Method | Path | 応答 |
|---|---|---|
| POST | `/webhook/github` | 署名不正・secret 未設定は 401 (body は読まない)。`release` 以外の event や `published` 以外の action は 204。記録したら 200 に event、既知の release なら 200 に `{"recorded": false}` |
| GET | `/v1/versions` | 全 repo の最新 `[{repo, tag, published_at, assets[{name,url,digest}], source, received_at}]` |
| GET | `/v1/versions/{org}/{repo}` | その repo の最新。無ければ 404 |
| GET | `/v1/events?since=<id>&limit=<n>` | `id > since` の event を id 昇順で最大 `limit` (既定 100、上限 500) |
| GET | `/v1/events/stream?since=<id>` | SSE。接続時に `since` 以降を流してから、新しい event が出るたびに送る。`id:` に event id、`event: release`、`data:` に event の JSON。再接続は最後の id を `since` に渡す（`Last-Event-ID` ヘッダーは参照しない） |
| GET | `/healthz`, `/api/health` | 生存確認 |

`source` は `webhook` か `poll`。`assets[].digest` は GitHub が返さないときは null。

## 環境変数

起動時に読むアプリ設定です。すべて省略して起動できますが、webhook を受け付けるには
secret、polling には監視 repo の指定が必要です。

| 変数 | 必須 / 任意 | 未設定時の既定値 | 用途・不正値の扱い |
| --- | --- | --- | --- |
| `PORT` | 任意 | `3000` | 全 IPv4 interface (`0.0.0.0`) の待受ポート。ASCII 数字の `1`–`65535` のみ。空、符号、空白、非数値、範囲外、非 Unicode 値は起動エラー。 |
| `VERSION_SERVER_DB` | 任意 | `data/version-server.db` | SQLite の path。相対パスは作業 directory 基準で、親 directory は作る。作成・open・初期化に失敗すると起動エラー。空文字は SQLite の一時 DB になるので永続化には使わない。 |
| `GITHUB_WEBHOOK_SECRET` | webhook 使用時に必要 | 無し | HMAC secret。空も未設定と同じで webhook は全部 401。空でない値はそのまま使い、送信元と一致しなければ署名検証で 401。 |
| `WATCH_REPOS` | polling 使用時に必要 | 無し（polling 無効） | `org/repo` の comma 区切り。各要素の前後空白と空要素は除く。名前の事前検証はなく、存在しない repo 等は polling 時に失敗を記録する。 |
| `GITHUB_TOKEN` | 任意 | 無し | polling の bearer。空も未設定扱い。有効性は起動時に検証せず、認証・権限エラーは polling 時に記録する。 |
| `POLL_SECS` | 任意 | `60` 秒 | polling 間隔。空・負数・数値として読めない値・u64 範囲外は `60`。`0` は背景 polling task が panic して停止するため、正の秒数を指定する。 |
| `GITHUB_API_URL` | 任意 | `https://api.github.com` | GHES 等の API base URL。末尾の `/` は除く。空や不正な URL は起動時に検証せず、polling 時に失敗を記録する。 |
| `RUST_LOG` | 任意 | `info` | tracing filter（例: `version_server=debug`）。不正な filter 構文・非 Unicode 値は `info`。空文字は有効な空 filter としてログを無効にする。 |

`PORT` 以外の文字列設定は、非 Unicode 値を未設定と同様に扱います。
`GITHUB_TOKEN` / `POLL_SECS` / `GITHUB_API_URL` は `WATCH_REPOS` に有効な要素がある場合だけ読みます。
polling のリクエスト失敗は HTTP server を停止せず、次の周期で再試行します。
`GITHUB_TOKEN` 無しでも public repo を監視できますが、rate limit は低くなります。

`PORT=3010 cargo run` で待受ポートを変更できます。旧 `APP_BIND_ADDR` は参照しません。
ログ設定は現在も `RUST_LOG` であり、`LOG_LEVEL` は読みません。
公開範囲は Compose / ingress 側で管理します。秘密は env で受け取り、値を log へ出しません。
読み取り元は [`src/main.rs`](src/main.rs)、polling の処理は [`src/github.rs`](src/github.rs) です。
Cargo / CI の build 用変数は runtime 設定ではありません。

## 配備 (home-server の compose に手で足す例)

```yaml
services:
  version-server:
    image: ghcr.io/miyabi-sunny-side/version-server:latest
    environment:
      - PORT=3000
      - VERSION_SERVER_DB=/app/data/version-server.db
      - GITHUB_WEBHOOK_SECRET=${VERSION_SERVER_WEBHOOK_SECRET}
      - GITHUB_TOKEN=${VERSION_SERVER_GITHUB_TOKEN}
      - WATCH_REPOS=miyabi-sunny-side/task-server,miyabi-sunny-side/task-worker
    volumes:
      - version-server-data:/app/data   # image は UID 10001 で動く
    ports:
      - "127.0.0.1:3010:3000"           # LAN への公開は既存の ingress で
    restart: unless-stopped
volumes:
  version-server-data:
```

上の `VERSION_SERVER_WEBHOOK_SECRET` / `VERSION_SERVER_GITHUB_TOKEN` は、この Compose 例が
共有 `.env` 内でサービスを識別する名前です。アプリは `GITHUB_WEBHOOK_SECRET` / `GITHUB_TOKEN`
として渡された値だけを読みます。実際のサービスごとの注入方法は
[home-server の README](https://github.com/miyabisun/home-server/blob/main/README.md) を参照してください。

GitHub 側は各 repo (または org) の webhook に `https://<公開 URL>/webhook/github`、content type `application/json`、secret に同じ値、event は `Releases` だけを選ぶ。cloudflared などで外から届く経路は home-server 側の設定で、本 repository の範囲外。

## 開発

```sh
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
npm --prefix client ci && npm --prefix client test && npm --prefix client run check
npm --prefix client run build && npm --prefix client run lint:design
cargo run   # http://127.0.0.1:3000 (client/dist を配信)
```

webhook を手で試す:

```sh
body='{"action":"published","release":{"tag_name":"v0.1.0","published_at":"2026-09-03T00:00:00Z","assets":[]},"repository":{"full_name":"o/r"}}'
sig=$(printf '%s' "$body" | openssl dgst -sha256 -hmac "$GITHUB_WEBHOOK_SECRET" | sed 's/^.* /sha256=/')
curl -sS -X POST localhost:3000/webhook/github -H "X-Hub-Signature-256: $sig" -H 'X-GitHub-Event: release' -H 'Content-Type: application/json' -d "$body"
```

## GitHub template and releases

rust-svelte-template から起こした。main・PR・手動 CI は frontend / Rust の検査と debug server の smoke を行う。PR・手動 CI は公開せず container build も検査する。

`Cargo.toml` と `Cargo.lock` の package version を更新して commit し、同じ commit に `v<major>.<minor>.<patch>` tag を付けて push すると Release container が動く。手動 release も同じ tag ref を選ぶ。tag commit の検査に成功し、tag と両 manifest の version が一致した場合だけ、そのソースから image を build / publish する。公開先は `ghcr.io/miyabi-sunny-side/version-server:<version>` と `:latest`。GitHub Release には、build 出力と一致を検証した `<version>@sha256:...` を記載する。

Docker は Rust 1.96.0 と cargo-chef 0.1.78 を固定し、frontend・Rust 依存・本体を別の層として構築する。tag 間の中間層は製品 image と別の `:build-cache` に registry cache (`mode=max`) として保存する。本体は実際の manifest で build するため、version だけの更新でも本体を再コンパイルする。

release profile は `opt-level=3` / `lto=false` / `codegen-units=16` / `strip=true`。軽微なサイズ・メモリ増を許容して build 待ち時間を減らす方針。

## License

MIT
