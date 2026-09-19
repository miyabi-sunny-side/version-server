# APIと設定

[導入と最初の確認](../README.md)

## API

| Method | Path | 応答 |
|---|---|---|
| POST | `/webhook/github` | 署名不正・secret未設定は401。署名検証に成功した後でJSONを解析する。`release` 以外の event や `published` 以外の action は 204。記録したら 200 に event、既知の release なら 200 に `{"recorded": false}` |
| GET | `/v1/versions` | 全 repo の最新 `[{repo, tag, published_at, assets[{name,url,digest}], source, received_at}]` |
| GET | `/v1/versions/{org}/{repo}` | その repo の最新。無ければ 404 |
| GET | `/v1/events?since=<id>&limit=<n>` | `id > since` の event を id 昇順で最大 `limit` (既定 100、上限 500) |
| GET | `/v1/events/stream?since=<id>` | SSE。接続時に `since` 以降を流してから、新しい event が出るたびに送る。`id:` に event id、`event: release`、`data:` に event の JSON。再接続時に最後のidを`since`へ渡す。`Last-Event-ID`ヘッダーは参照しない |
| GET | `/healthz`, `/api/health` | 生存確認 |

`source`は`webhook`か`poll`。GitHubがdigestを返さない場合、`assets[].digest`はnullになる。

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
読み取り元は [`src/main.rs`](../src/main.rs)、polling の処理は [`src/github.rs`](../src/github.rs) です。
Cargo / CI の build 用変数は runtime 設定ではありません。

## 保存する更新

Webhookとpollingは同じ保存処理を使います。同じtagの再受信では更新イベントを増やしません。
保存済みより古い`published_at`を持つ別tagが届いても、最新releaseは巻き戻しません。
時刻がない場合や解析できない場合は新旧を比較できません。

最新状態は`releases`、変更履歴は追記型の`events`へ保存します。
内部の処理は[Store::ingest](../src/store.rs)を参照してください。
