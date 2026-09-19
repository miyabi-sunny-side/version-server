# version-server

GitHubリポジトリの最新releaseを監視し、ブラウザとAPIで確認できるサーバーです。
定期取得と署名付きWebhookの両方に対応します。
他の端末は、このサーバーから最新tagや更新イベントを取得できます。

## Docker Composeで起動する

DockerとDocker Composeが必要です。次の内容を`compose.yaml`として保存します。
`WATCH_REPOS`は監視したい公開リポジトリの`org/repo`へ置き換えてください。
複数指定する場合はカンマで区切ります。例では`rust-lang/rust`を監視します。

```yaml
services:
  version-server:
    image: ghcr.io/miyabi-sunny-side/version-server:latest
    environment:
      PORT: "3000"
      VERSION_SERVER_DB: /app/data/version-server.db
      WATCH_REPOS: rust-lang/rust
      GITHUB_TOKEN: "${GITHUB_TOKEN:-}"
    volumes:
      - version-server-data:/app/data
    ports:
      - "127.0.0.1:3010:3000"
    restart: unless-stopped
volumes:
  version-server-data:
```

```sh
docker compose up -d
```

`http://127.0.0.1:3010`を開くと、取得したreleaseのtagと受信時刻が表示されます。
取得は起動後に始まり、既定では60秒ごとに確認します。
表示が空のままなら、リポジトリに公開releaseがあることとログを確認してください。

```sh
docker compose logs version-server
curl --fail http://127.0.0.1:3010/v1/versions
```

APIはリポジトリごとの`repo`、`tag`、`published_at`、`assets`などを返します。
`WATCH_REPOS`を変更したら、`docker compose up -d`でコンテナへ反映します。
取得済みのデータは名前付きvolumeに残るため、更新時もこのvolumeを保持してください。
監視対象から外しても、保存済みのreleaseは削除されません。

公開リポジトリはtokenなしでも監視できますが、GitHubのrate limitを受けます。
必要なら、起動するシェルやCompose用の`.env`へ`GITHUB_TOKEN`を設定します。
秘密値をリポジトリへ保存しないでください。アプリ自身は`.env`を読みません。

## Webhookを併用する

pollingだけで利用できます。即時に受信したい場合はWebhookを設定します。

1. サーバーへ空でない`GITHUB_WEBHOOK_SECRET`を環境変数として渡します。
   Composeでは`environment`に`GITHUB_WEBHOOK_SECRET: "${GITHUB_WEBHOOK_SECRET}"`を追加します。
2. GitHubの対象リポジトリまたは組織でWebhookを作成します。
   URLは`https://<公開ホスト>/webhook/github`、Content typeは`application/json`です。
3. 同じsecretを設定し、イベントは`Releases`を選択します。

`published`のreleaseを受け付けます。secret未設定や署名不正は401で拒否します。
GitHubからWebhookへ届くHTTPS経路は、プロキシなど配備側で用意してください。

## 保存とアクセス範囲

最新releaseと更新イベントをSQLiteへ保存します。データ用volumeはコンテナUID `10001`が使います。
ホストのディレクトリをマウントする場合は、書込み権限を合わせてください。
バックアップにはSQLiteのbackup機能を使うか、サービスを停止してデータ全体をコピーします。

読み取りAPIと画面には認証がありません。上の例はlocalhostだけに公開します。
LANやリモート端末から使う場合は、ネットワークまたはプロキシで到達範囲を管理してください。
Webhookの署名検証は読み取りAPIへのアクセス制御にはなりません。

## 詳細

- [APIと設定](docs/reference.md): 環境変数、最新version取得、イベントとSSE。
- [開発とリリース](docs/development.md): ソースからの起動、検証、Webhookの試験、配布手順。

ライセンスは[MIT](LICENSE)です。
