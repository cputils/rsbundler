# rsbundler

[English](README.md) | 日本語

**rsbundler** は、Rust のクレートルートとローカルなソースファイル依存関係を、1 つの Rust ソースへまとめます。構文解析器は実行ファイルへリンクされており、バンドル時に `rustc`、`cargo`、`rustfmt`、その他の外部コマンドを起動しません。

## クイックスタート

リポジトリからコマンドをインストールします。

```sh
cargo install --git https://github.com/cputils/rsbundler rsbundler
```

バイナリクレートのルートをバンドルします。

```sh
rsbundler src/main.rs --output bundled.rs
```

`--output` を省略すると、生成ソースは標準出力へ書き込まれます。

```sh
rsbundler src/main.rs > bundled.rs
```

`rsbundler` の実行時に Rust ツールチェーンは不要です。当然ながら、生成した `.rs` を後からコンパイルするときには Rust コンパイラが必要です。

共有system libraryにも依存しない、完全静的なx86-64 Linux実行ファイルはrepository taskでbuildできます。

```sh
mise run build-static-linux-x86_64
```

成果物は `target/x86_64-unknown-linux-musl/release/rsbundler` です。Rustとthird-party libraryのコードは、1つのstatic PIE実行ファイルにlinkされます。完全静的なmusl processからCargo projectのhost proc-macro dylibはloadできないため、このbuildは自動検出した成果物をskipし、明示的な `--proc-macro` overrideをrejectして、その呼び出しを文書化済みの未展開マクロ境界内に残します。projectのproc-macro成果物をprocess内で展開する必要がある場合は通常のhost buildを使ってください。どのbuildもbundle中に外部compiler、formatter、macro server、language runtimeを起動しません。

## 動作原理

1. 実行ファイルへ組み込んだ rust-analyzer の構文解析器でクレートルートを解析します。
2. Cargoプロジェクトの既存build成果物からコンパイル済み手続き型マクロライブラリを検出し、属性・derive・関数形式マクロを同一process内で展開します。
3. Rust のファイル解決規則に従って、外部ファイルの `mod name;` 宣言をたどります。
4. 各宣言のセミコロンだけを inline module の本体へ置換し、可視性、属性、コメント、元のソーステキストを維持します。
5. デフォルトでは、静的な `include!`、`include_str!`、`include_bytes!` の参照ファイルも埋め込みます。

抽象構文木からソース全体を再出力する方式ではありません。展開に必要な範囲だけを置換するため、それ以外のコメント、raw 文字列、空白、改行コードはそのまま保たれます。

## 対応するソース解決

- `name.rs` と `name/mod.rs`（両方が存在する場合の曖昧性検出を含む）
- flat module、`mod.rs`、inline module 内のネストした外部ファイル module
- `mod r#type;` のような raw identifier
- `#[path = "..."]` と有効な `cfg_attr(..., path = "...")`（通常とは異なる子 module の解決規則も含む）
- Rust item 群または 1 つの式を含む、静的でネスト可能な `include!`
- 任意の UTF-8 テキストに対する `include_str!` と、任意のバイト列に対する `include_bytes!`
- リテラル、`concat!`、`env!`、`file!`、コメントや内部空白を含まない `stringify!` からなる include パス（`concat!` 内の `line!` と `column!` も対応）
- `all()`、`any()`、補集合によって定数へ簡約できるpredicateなど、`concat!` 内のtarget非依存な `cfg!` 値
- `concat!` 内の文字列、文字、整数（非10進・suffix付きも含む）、浮動小数点数、真偽値、負数リテラル
- `std::include_str!` のような修飾済み標準マクロと、`use std::include_str as embedded` のような明示的alias
- 明示的な標準aliasを含む直接の `file!`、`line!`、`column!`（ソースを移動する前に元の値へ置換）
- `macro_rules!` transcriber 内の、静的にparseできるmodule / include依存
- Cargo build成果物から自動検出するか、host dylibで明示的に上書きした属性・derive・関数形式の手続き型マクロ
- `cfg`、`all`、`any`、`not`、ネストした `cfg_attr` 分岐のターゲット非依存な展開
- UTF-8 BOM とソースファイルの shebang
- rollback可能な循環参照検出と、依存ソースファイル数の上限

外部 crate と `use` 宣言は変更しません。Cargo metadata を実行したり、サードパーティー crate をソースへコピーしたりすることもありません。

## 手続き型マクロ

通常、proc-macro用のoptionは不要です。先にCargoプロジェクトをbuildしてから、crate rootをbundleします。

```sh
cargo build
rsbundler src/main.rs --output bundled.rs
```

rsbundlerはentryファイルを起点に、所属packageとworkspaceのmanifestを検出し、dependencyのrenameを読み取り、既存のCargo成果物directoryを探索します。`CARGO_TARGET_DIR` と `[build].target-dir` に対応し、それらがなければpackageとworkspaceの `target` directoryをdebug、release、custom profileの順で探索します。Cargoやrustcを起動せず、欠けている成果物をbuildすることもありません。

非標準の配置を使う場合や自動選択した成果物を上書きする場合は、コンパイル済みproc-macro dylibを `CRATE=DYLIB` 形式で指定します。crate名は呼び出し元ソースで使うpathの先頭segmentです。このoptionは複数回指定できます。

```sh
rsbundler src/main.rs \
  --proc-macro my_macros=target/debug/deps/libmy_macros-abc123.so \
  --output bundled.rs
```

rsbundlerは各ライブラリからmacro exportを読み取り、通常のソース依存探索より前に、登録済みの属性マクロ、custom derive、関数形式マクロを展開します。macro出力も再帰的に処理するため、生成された `mod`、`include!`、別の登録済み手続き型マクロも同じbundleへ取り込まれます。`#[my_macros::transform]` のような修飾済み呼び出しは、検出したdependency名または明示的な上書き名で照合します。未修飾名は、その種類でロード済みのmacroを一意に特定できる場合に展開します。衝突する場合はcrate pathで修飾してください。

Rustのproc-macro dylib ABIは不安定なため、dylibはhost platform向けで、rsbundler実行ファイルと完全に同じ `rustc` versionでコンパイルされている必要があります。明示指定したdylibのABIが一致しない場合は、期待versionと実際のversionを示して拒否し、自動検出した非互換の候補はskipします。rsbundlerはCargo、rustc、外部proc-macro serverを起動しません。proc-macro host、成果物探索、それらのRust依存ライブラリはrsbundlerバイナリへリンクされ、実行時にロードするのはプロジェクト内のdylibだけです。手続き型マクロはnative codeとしてrsbundlerと同じ権限で動作するため、信頼できるプロジェクトと成果物だけをbundleしてください。

`--env KEY=VALUE` の値は手続き型マクロにも渡します。current directoryはentryソースのdirectoryです。`cfg_attr`から間接的に選ばれるmacro、compilerが提供するderive、完全なcompiler名前解決を必要とするmacroは、この構文的展開modelの対象外です。未展開の属性には、従来どおり保守的なmodule保持を適用します。

## バンドル制御

pybundler と同様に、通常コメントの同じ行、または単独コメントとして直前の行にある `bundle` / `no-bundle` を認識します。

```rust
mod embedded; // bundle

// no-bundle
mod generated_at_build_time;

const SCHEMA: &str = include_str!(SCHEMA_PATH); // no-bundle
```

`// no-bundle` は宣言またはマクロ呼び出しを必ず変更せず残します。その構文を移動すると相対パスやソース位置が変わる場合は、直近の外側のファイル依存も保持します。`// bundle` は静的展開を強制し、`--external` より優先され、依存を解決できなければエラーを返します。内側の `// no-bundle` は、外側の依存に指定した `// bundle` よりも優先されます。同じ依存に両方を指定するとエラーです。どちらもない場合、安全に解決できる依存は展開し、非対応・欠落・曖昧・動的計算・循環している依存はトランザクション単位で巻き戻して、エラーにせず保持します。

`-e, --external <MODULE>` はトップレベルmoduleとその配下を保持します。複数指定でき、`name` と `crate::name` の両方を受け付けます。その宣言に `// bundle` があれば、moduleと推移的なローカルmoduleを取り込みます。

### 条件付きコンパイル

rsbundler はコンパイル対象を選択せず、自身をコンパイルした際の cfg 値も使用しません。元の条件属性を残したまま、静的に解決できるすべての依存分岐を展開します。`any()` のように常に偽となる述語は、依存ファイルを読まずに除外します。includeパス内の `cfg!` は、boolean簡約によってcompile構成に依存しない値だと証明できる場合だけ評価し、target依存の形は変更せず残します。

`cfg_attr(..., path = "...")` が1つのmoduleに対して異なるファイルを選択し得る場合、選択される各pathとデフォルトmodule pathを、互いに排他的な `#[cfg(...)]` 付きinline module分岐として出力します。これにより、Rustコンパイラが任意のcfg一式で利用できる単一のbundleになります。欠落・曖昧・循環・生成予定などの理由で安全に展開できない候補は、その条件下だけ元の外部ファイルmodule宣言として残し、ほかの候補は展開します。複数のpath属性が同時に有効になり得る重複条件も変更せず残し、診断や優先順位はrustc自身に委ねます。ネストした `cfg_attr`、条件付き属性マクロ、pathによって探索基準が変わるinline ancestor、候補ごとに異なる子moduleの基準も同様に扱います。

## 精度上の境界

コンパイラ相当の完全な名前解決を行わない限り、次の動作は完全には再現できません。

- metavariableを含む `macro_rules!` transcriberが、呼び出しや周囲のscopeに応じて依存を選ぶ場合
- 未設定または条件付きで選択される手続き型マクロが依存を生成したり、inline moduleと外部ファイルmoduleを区別したりする場合
- 任意のユーザー定義・外部マクロがincludeやソース位置依存の組み込みマクロへ展開される場合
- entryファイル内のマクロに位置依存マクロが隠れており、保持できる外側のファイル依存がない場合

ユーザー定義・import済みのマクロ名は、検出した `#[macro_export]` 定義と直接または条件付きの `macro_use` 属性を含めて保守的に追跡します。シャドーイングの可能性がある未修飾のinclude、静的パス、位置マクロは保持し、修飾済み標準パスと標準aliasは、その `std` / `core` prefixが利用可能でシャドーイングされていない場合だけ展開します。`macro_rules!` transcriber内のparse可能な依存構文は展開し、context依存のtranscriberは移動が危険な場合に外側の依存ごと保持します。未知の属性マクロが付いたmoduleも自動では保持します。認識された構文をinline化して安全だと利用者が保証できる場合にだけ `// bundle` を追加してください。

直接の `file!`、`line!`、`column!` は元の値を正確に維持します。標準aliasや `concat!` を介した場合も含め、認識できる静的includeパス内の同じマクロは元の呼び出し位置で評価します。位置マクロまたはinclude呼び出しがそれ以外のマクロ入力内にある場合、移動によってソース位置または相対パスの基準が変わり得るmodule / includeを外側から保持します。crate entryには保持できる親依存がないため、別の隠れた位置マクロがあればentry自体を変更せず返します。このテキストを別のファイル名でコンパイルすると、隠れた `file!` の値だけは変わり得ます。この場合を完全に再現するにはユーザーマクロの展開が必要です。

保持された `mod` とinclude構文は、引き続き元のファイルへ依存します。部分的なinline化で相対パスの基準が変わる場合、rsbundlerはより外側の依存を保持しますが、トップレベルで保持されたパスは生成ファイルの配置場所を基準に解釈されます。`no-bundle`、`external`、自動保持を使う場合は、entryの隣へ出力するか、必要な相対配置を維持してください。すべて展開できた生成物には、このローカルなビルド時ソース依存はありません。

pybundler のinterpreter / sys.path境界オプションは、すべてのRustファイルmoduleが明示的に宣言されるため対応物がありません。Python importのtree shakingも、Rustコンパイラが不要コードを除去し、ソース段階の削除は安全でないため実装しません。formatは `rustfmt` の起動が外部ランタイム非依存の保証に反するため行いません。Cargo依存をコピーしないため、パッケージlicenseの埋め込みも対象外です。

## CLI オプション

| オプション                   | 説明                                                                | デフォルト  |
| ---------------------------- | ------------------------------------------------------------------- | ----------- |
| `-o, --output <FILE>`        | 標準出力ではなくファイルへ書き込む                                  | 標準出力    |
| `--edition <EDITION>`        | Rust `2015`、`2018`、`2021`、`2024` のいずれかとして解析する        | `2024`      |
| `--max-source-files <COUNT>` | entry を除く、展開する module / include ファイル数を制限する        | `2048`      |
| `-e, --external <MODULE>`    | トップレベルmoduleを保持する（複数指定可）                          | なし        |
| `--env <KEY=VALUE>`          | 静的includeパスの `env!` 値と手続き型マクロの環境overrideを設定する | process環境 |
| `--proc-macro <CRATE=DYLIB>` | 自動検出した手続き型マクロを上書き（複数指定可）                    | 自動検出    |
| `--no-inline-includes`       | 3 種類の組み込み include マクロを生成コードへ残す                   | 無効        |

完全なヘルプは `rsbundler --help` で確認できます。

## Rust ライブラリ

`Cargo.toml` に rsbundler を追加します。

```toml
[dependencies]
rsbundler = { git = "https://github.com/cputils/rsbundler", tag = "<version>" }
```

次に `bundle_file` を呼び出します。

```rust
use rsbundler::{BundleOptions, bundle_file};

let result = bundle_file("src/main.rs", BundleOptions::default())?;
std::fs::write("bundled.rs", result.code)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`BundleResult` には、正規化された entry の絶対パスと、バンドルに使ったすべての module / include ファイルの決定的なメタデータも含まれます。

`BundleOptions` にはedition、ファイル数上限、include制御に加え、対応する `external`、`environment`、`proc_macros` フィールドがあります。defaultでは既存のCargo成果物を自動検出し、source上のcrate pathとコンパイル済みライブラリを上書きするときだけ `ProcMacroDylib` を追加します。optionsで指定した環境値はrsbundler processの環境変数より優先され、`CARGO_TARGET_DIR` の探索と手続き型マクロの展開時にも参照できます。

## ライセンス

MIT
