# RESONANCE BUNNYHOP

Windows 向けのコントローラ入力補助ツールです。  
物理コントローラの入力を読み取り、`ViGEmBus` 経由で仮想 `Xbox 360 Controller` を作成し、ジャンプ前後の移動入力タイミングを補助します。

## 動作確認済み環境

- Windows 10 / 11
- DualSense
  - USB: 動作確認済み
  - Bluetooth: 動作確認済み
- Nintendo Switch Pro Controller
  - USB: 動作確認済み

## 動作保証について

- 動作確認できているのは `DualSense の USB / Bluetooth` と `Nintendo Switch Pro Controller の USB` のみです。
- そのほかのコントローラは、対応している可能性はありますが、動作保証外です。
- サードパーティ製コントローラ、中華パッド、変換アダプタ経由、独自ドライバ経由の環境では正常に動かない場合があります。

## インストール時の警告について

- `ViGEmBus`
- `HidHide`
- `RESONANCE BUNNYHOP`

はいずれも、インストール時または初回実行時に Windows から警告が出る場合があります。

- Windows の保護機能から 2 回程度確認が出る場合があります。
- 利用しているウイルス対策ソフトによっては、さらに追加の警告が出る場合があります。
- これはドライバ導入や未署名・認知度の低い実行ファイルで起こりやすい挙動です。

配布物を実行する前に、配布元とファイル内容を必ず確認したうえで利用してください。

## 導入に必要なもの

このツールを使うには、次の 2 つが必要です。
### 1. ViGEmBus

`ViGEmBus` は、Windows 上に仮想 `Xbox 360 Controller` を作るためのドライバです。  
このツールは、補助後の入力を `ViGEmBus` 経由でゲームへ送ります。

注意:

- `ViGEmBus` はプロジェクト終了済みですが、このツールの動作には現状必要です。

導入手順:

1. [ViGEmBus の Releases ページ](https://github.com/nefarius/ViGEmBus/releases) を開きます。
2. 一番上のリリースを開き、`Assets` から `ViGEmBus_1.22.0_x64_x86_arm64.exe` をダウンロードします。
3. ダウンロードしたセットアップを実行してインストールします。
2. インストール後、必要に応じて Windows を再起動します。

### 2. HidHide

`HidHide` は、物理コントローラをゲームから隠すためのドライバです。  
このツールは物理コントローラを直接読みますが、ゲーム側には仮想 `Xbox 360 Controller` だけを見せるのが基本構成です。

導入手順:

1. [HidHide の Releases ページ](https://github.com/nefarius/HidHide/releases) を開きます。
2. 一番上のリリースを開き、`Assets` から`HidHide_1.5.230_x64.exe`をダウンロードします。
3. ダウンロードしたセットアップを実行してインストールします。
2. `HidHide Configuration Client` を起動します。
3. `Applications` に `resonance-bhop.exe` を追加します。
4. `Devices` で、ゲームから隠したい物理コントローラにチェックを入れます。
5. `Enable device hiding` をオンにします。

## なぜ ViGEmBus と HidHide の両方が必要なのか

- `ViGEmBus`
  - このツールが補助後の入力を送るために必要です。
  - これがないと、仮想 `Xbox 360 Controller` を作れません。

- `HidHide`
  - 物理コントローラをゲームから隠すために必要です。
  - これがないと、ゲームが
    - 物理コントローラの入力
    - このツールが作った仮想 Xbox コントローラの入力
    の両方を同時に見てしまう場合があります。
  - その状態だと、一瞬ニュートラルにする補助や入力差し替えが打ち消され、補助が効かなくなることがあります。

要するに、

- `ViGEmBus` = 仮想コントローラを作るために必要
- `HidHide` = 物理コントローラをゲームから隠すために必要

です。

## 使い方

1. `ViGEmBus` と `HidHide` を導入します。
2. `HidHide` に `resonance-bhop.exe` を登録します。
3. `HidHide` の `Devices` で、使う物理コントローラにチェックを入れます。
4. `resonance-bhop.exe` を起動します。
5. 必要なら `オプション` で入力元やジャンプボタン、閾値を調整します。
6. `開始` を押してブリッジを有効化します。

## ジャンプボタン設定について

- `ジャンプボタン設定` の `入力から設定` は、`BRIDGE` をオンにした状態で使ってください。
- `BRIDGE` がオフの間は、このツールは物理入力を読みに行かないため、入力から設定は機能しません。

## Bluetooth 利用時の注意

- `DualSense Bluetooth` は動作確認済みです。
- Bluetooth は USB より不安定なことがあります。
- 補助が効かない、接続直後の挙動がおかしい、コントローラが正しく見えない、という場合は次を試してください。

1. コントローラの Bluetooth 接続を一度切る
2. もう一度接続する
3. ツールを再起動する
4. それでもダメなら、Windows の Bluetooth 設定から一度削除して再ペアリングする

## HidHide 利用時の注意

- 同じコントローラでも、`USB` と `Bluetooth` は別デバイスとして見えることがあります。
- USB では隠せていても、Bluetooth 側は別途チェックが必要な場合があります。
- `resonance-bhop.exe` 自体を `Applications` に入れ忘れると、このツールからも物理コントローラが見えなくなる場合があります。

## うまく動かないとき

次の順で確認すると切り分けしやすいです。

1. `ViGEmBus` がインストールされているか
2. `HidHide` がインストールされているか
3. `HidHide` の `Applications` に `resonance-bhop.exe` が登録されているか
4. `HidHide` の `Devices` で対象コントローラにチェックが入っているか
5. `Enable device hiding` がオンになっているか
6. ツールの `入力元` が正しいか
7. コントローラを抜き差し、または Bluetooth 再接続してみる
8. ツールを再起動してみる

それでも改善しない場合は、次の常駐ソフトや入力変換ソフトが競合している可能性があります。

- Steam Input
- DS4Windows
- DualSenseX
- そのほかの仮想パッド / 入力変換ソフト

このツールを使うときは、基本的にそれらを止めることをおすすめします。

## 既知の注意点

- ゲームやアンチチート環境によっては、仮想入力が正しく使えない場合があります。
- Bluetooth 接続では、再接続で直るケースがあります。
- Nintendo Switch Pro Controller の Bluetooth は、この配布時点では動作確認対象外です。
- 動かないコントローラは、現時点では対応外の可能性があります。

## 免責

本ソフトウェアは現状のまま提供されます。  
利用によって発生した不具合、損害、アカウント制限等について、作者は責任を負いません。

## ライセンス

このリポジトリは `MIT License` です。詳細は [LICENSE](https://github.com/Defoco303/resonance-bunnyhop/blob/main/LICENSE)を参照してください。
