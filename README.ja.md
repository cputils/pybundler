# pybundler

![Badge](https://github.com/cputils/pybundler/actions/workflows/ci.yml/badge.svg)

[English](README.md) | 日本語

**pybundler** は、Python のエントリーファイルとそのローカル依存関係を、単一の自己完結した Python スクリプトにまとめます。

## クイックスタート

リポジトリから `pybundler` コマンドをインストールします。

```sh
cargo install --git https://github.com/cputils/pybundler pybundler
```

プログラムをバンドルします。

```sh
pybundler src/main.py --output bundled.py
python bundled.py
```

`--output` を指定しない場合、生成されたスクリプトは標準出力に書き込まれます。

```sh
pybundler src/main.py > bundled.py
```

すべてのオプションを確認するには、`pybundler --help` を使用してください。

## 詳細リファレンス

### バンドルの仕組み

1. pybundler はエントリーファイルを解析し、ローカルインポートを再帰的にたどります。
2. CPython と互換性のあるパッケージおよびパスエントリの優先順位に従って、それらのインポートを解決します。
3. バンドルする各モジュールを読みやすい `if __name__ == "...":` ソースブロックとして出力し、軽量なインポートランタイムを追加します。
4. 生成されたスクリプトは、バンドル元のソースファイルがなくても実行できます。

生成されるランタイムは、パッケージのメタデータと循環インポートの動作を維持します。名前空間パッケージの親に `__init__.py` がない場合は、必要に応じて合成します。

### 対応するインポートとモジュール解決

pybundler は以下に対応しています。

- `import`、`from ... import`、別名、相対インポート、ワイルドカードインポート
- 引数がコンパイル時定数である `__import__()` と `importlib.import_module()`（静的な `fromlist` の値を含む）
- `__all__` が静的に宣言されたパッケージからのワイルドカードインポート
- 通常のパッケージ、名前空間パッケージ、モジュール、ディレクトリ、ZIP ベースのパスエントリ
- 任意のアーカイブ拡張子と内部パスのプレフィックスを持つ ZIP パスエントリ
- 対応するすべてのモジュールおよびパスエントリ形式における CPython の検索優先順位

デフォルトでは、エントリーファイルのディレクトリから解決を開始します。CLI はさらに、一般的な Python コマンドを使用して追加の `sys.path` エントリを検出します。インストールされていない、または実行できないコマンドは無視されます。Unix 系システムでは `python3`、`python`、`pypy3`、`pypy` を試し、Windows では最初に `py` ランチャーも試します。1 つ以上の `--interpreter` の値を渡すと、これらのデフォルトは置き換えられます。

インタープリターの `sys.path` から解決されたインポートには、`--no-require-bundle-directive` を使用しない限り、プロジェクト境界で `# bundle` ディレクティブが必要です。一度取り込まれると、その推移的なインポートは追加のディレクティブなしでバンドルされます。

`--external` で指定したトップレベルパッケージのインポートは、実行時インポートのまま維持されます。個々のインポートに付けた `# bundle` ディレクティブはこの設定を上書きし、`# no-bundle` はそのインポートがバンドルされるのを防ぎます。

### インポートの制御

パッケージをバンドルせず、通常の実行時依存関係として維持するには、次のようにします。

```sh
pybundler src/main.py --output bundled.py --external numpy
```

インストール済みパッケージをバンドルするには、そのインポートに `# bundle` を付けます。

```python
import some_package  # bundle
```

プラットフォームのデフォルトではなく、特定の Python 環境を選択する必要がある場合は、`--interpreter <COMMAND>` を使用します。

通常の実行時インポートとして維持するインポートには、`# no-bundle` を追加します。

```python
import another_package  # no-bundle
```

### ソース処理と出力

- 未使用のインポートはデフォルトで削除されます。`# bundle` が付いたインポートは維持されます。
- `--no-tree-shaking` を指定すると、未使用のインポートが維持されます。
- 結合後のスクリプトを有効な Python として維持するため、バンドルされたモジュールの `__future__` インポートは先頭に移動されます。
- UTF-8 BOM、および UTF-8、ASCII、Latin-1 の PEP 263 宣言に対応しています。ソースは UTF-8 として出力され、エンコーディング宣言もそれに合わせて正規化されます。
- バンドルされたサードパーティーパッケージ内で検出されたライセンステキストは、自動的に埋め込まれます。
- `--format` を指定すると、生成されたスクリプトが Ruff でフォーマットされます。
- `--max-imported-modules` は依存グラフの展開数を制限し、デフォルトは `2048` です。

ネイティブ拡張モジュール、ソースのないバイトコードモジュール、および UTF-8、ASCII、Latin-1 以外のインタープリター登録済みコーデックを使用するソースファイルは、実行時依存関係として維持されます。これらを再現するには、対象の Python インタープリターまたはプラットフォームが必要です。

### CLI オプション

「複数指定可」と記載されたオプションは、複数回指定できます。

| オプション                       | 説明                                                                  | デフォルト                         |
| -------------------------------- | --------------------------------------------------------------------- | ---------------------------------- |
| `-o, --output <FILE>`            | バンドルを標準出力ではなくファイルに書き込む                          | 標準出力                           |
| `-e, --external <PACKAGE>`       | トップレベルパッケージを実行時インポートとして維持する（複数指定可）  | なし                               |
| `--max-imported-modules <COUNT>` | バンドルするインポート済みモジュールの数を制限する                    | `2048`                             |
| `-i, --interpreter <COMMAND>`    | Python インタープリターを使用して `sys.path` を検出する（複数指定可） | プラットフォームで一般的なコマンド |
| `--no-require-bundle-directive`  | `sys.path` 経由で見つかったインポートを `# bundle` なしでバンドルする | 無効                               |
| `--no-tree-shaking`              | バンドルされたモジュール内の未使用インポートを維持する                | 無効                               |
| `--format`                       | 生成されたバンドルを Ruff でフォーマットする                          | 無効                               |

### Rust ライブラリ

`Cargo.toml` に pybundler を追加します。

```toml
[dependencies]
pybundler = { git = "https://github.com/cputils/pybundler", tag = "<version>" }
```

次に、`bundle_file` を呼び出します。

```rust
use pybundler::{BundleOptions, bundle_file};

let result = bundle_file("src/main.py", BundleOptions::default())?;
std::fs::write("bundled.py", result.code)?;
```

`BundleOptions` は、CLI 設定に相当するライブラリ用の設定を提供します。CLI と異なり、ライブラリはデフォルトではインタープリターを選択しません。

| フィールド                 | 説明                                                            | デフォルト |
| -------------------------- | --------------------------------------------------------------- | ---------- |
| `external`                 | 実行時インポートとして維持するトップレベルパッケージ            | `[]`       |
| `max_imported_modules`     | バンドルするインポート済みモジュールの最大数                    | `2048`     |
| `interpreter`              | `sys.path` の検出に使用する Python インタープリター             | `[]`       |
| `require_bundle_directive` | `sys.path` 経由で解決されたインポートに `# bundle` を必須とする | `true`     |
| `tree_shaking`             | `# bundle` が付いていない未使用のインポートを削除する           | `true`     |
| `format`                   | バンドルされた出力を Ruff でフォーマットする                    | `false`    |

`bundle_file` は、生成された `code`、エントリーファイルとエントリーモジュールの情報、およびバンドルされた各モジュールのメタデータを含む `BundleResult` を返します。

## ライセンス

MIT
