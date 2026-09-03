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
| GET | `/v1/events/stream?since=<id>` | SSE。接続時に `since` 以降を流してから、新しい event が出るたびに送る。`id:` に event id、`event: release`、`data:` に event の JSON。再接続は最後の id を `since` に渡す |
| GET | `/healthz`, `/api/health` | 生存確認 |

`source` は `webhook` か `poll`。`assets[].digest` は GitHub が返さないときは null。

## 設定 (env)

| 変数 | 既定 | 用途 |
|---|---|---|
| `APP_BIND_ADDR` | `127.0.0.1:3000` | listen address (container では `0.0.0.0:3000`) |
| `VERSION_SERVER_DB` | `data/version-server.db` | SQLite の path。親 directory は作る |
| `GITHUB_WEBHOOK_SECRET` | (無し) | webhook の HMAC secret。無ければ webhook は全部 401 |
| `WATCH_REPOS` | (無し) | polling する `org/repo` の comma 区切り。空なら polling しない |
| `GITHUB_TOKEN` | (無し) | polling の bearer。無くても public repo は読めるが rate limit が低い |
| `POLL_SECS` | `60` | polling の間隔 |
| `GITHUB_API_URL` | `https://api.github.com` | テストや GHES 向けの差し替え口 |

秘密は env だけで受け取り、log には有無しか出さない。

## 配備 (home-server の compose に手で足す例)

```yaml
services:
  version-server:
    image: ghcr.io/miyabi-sunny-side/version-server:latest
    environment:
      - APP_BIND_ADDR=0.0.0.0:3000
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

rust-svelte-template から起こした。`Dockerfile` と `.github/workflows` は template のまま: CI が `sha-<short>` の image を push し、tag の Release container workflow が `<version>` / `latest` に retag して GitHub Release を作る。release の手順は template の README に従う。

## License

MIT
