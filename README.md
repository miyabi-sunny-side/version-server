# version-server

家庭内 product の「いま公開されている最新 version」を 1 か所で監視し、LAN 内へ配る。外 (GitHub、必要なら Gmail) を見るのはこの server だけで、worker 機は秘密を持たずにここへ問い合わせる。

- 入力: GitHub webhook の `release` event (cloudflared 経由)。保険として GitHub Releases API の polling
- 状態: repo ごとの最新 release (tag、published_at、asset の URL と digest)
- 出力: `GET /v1/versions`、`GET /v1/versions/<org>/<repo>`、`GET /v1/events?since=<id>` (SSE)。LAN 内なので短い間隔の polling や張りっぱなしの接続を許す
- 消費者: sandbox の release-listener (task-worker の更新と watchtower の pull を起動)、task-worker の自己更新
