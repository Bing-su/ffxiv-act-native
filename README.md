# ffxiv-act-native

Rust `cdylib`으로 FFXIV ACT 플러그인을 작성할 수 있도록 AnyCPU .NET Framework
shim을 생성합니다. 생성기는 `Advanced Combat Tracker.exe`와 공식 SDK의
`FFXIV_ACT_Plugin.Common.dll`만 읽으며 `FFXIV_ACT_Plugin.dll`은 입력이나 참조로
사용하지 않습니다.

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
ffxiv-act-native = "0.1"
```

```rust
use ffxiv_act_native::{
    Event, Plugin, PluginInit, PluginResult, Repository, SubscriptionSet, export_plugin,
};

struct MyPlugin;

impl Plugin for MyPlugin {
    fn init(repository: Repository<'_>) -> PluginResult<PluginInit<Self>> {
        let _player_id = repository.current_player_id()?;
        Ok(PluginInit::new(
            Self,
            SubscriptionSet::ZONE_CHANGED | SubscriptionSet::LOG_LINE,
        ))
    }

    fn on_event(&mut self, _: Repository<'_>, event: Event<'_>) -> PluginResult<()> {
        eprintln!("{event:?}");
        Ok(())
    }

    fn shutdown(&mut self, _: Repository<'_>) -> PluginResult<()> { Ok(()) }
}

export_plugin!(MyPlugin);
```

`embedded-contracts` 피처를 build dependency에서 활성화하면 ACT나 SDK 설치 없이
shim을 함께 빌드할 수 있습니다.

```toml
[build-dependencies]
ffxiv-act-native = { version = "0.1", features = ["embedded-contracts"] }
```

```rust,no_run
use ffxiv_act_native::{PluginConfig, PluginMetadata};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    ffxiv_act_native::build_shim(&PluginConfig {
        assembly_name: "MyPlugin.ACT".into(),
        native_dll_name: "my_plugin.dll".into(),
        metadata: PluginMetadata {
            file_version: [0, 1, 0, 0],
            product_version: "0.1.0".into(),
            file_description: "My ACT plugin".into(),
            product_name: "MyPlugin".into(),
            ..Default::default()
        },
    })
}
```

`PluginMetadata`는 assembly/file/product 버전과 파일 설명, 제품명, 회사명,
저작권 및 설명을 설정합니다. `OriginalFilename`은 assembly 이름에서 생성됩니다.
직접 입력 어셈블리를 지정해야 하는 경우에는 저수준 생성 API를 사용합니다.

```rust,no_run
use ffxiv_act_native::{PluginConfig, PluginMetadata, generate};

let act = std::fs::read(r"C:\Program Files\Advanced Combat Tracker\Advanced Combat Tracker.exe")?;
let common = std::fs::read(r"sdk\FFXIV_ACT_Plugin.Common.dll")?;
let shim = generate(&act, &common, &PluginConfig {
    assembly_name: "MyPlugin.ACT".into(),
    native_dll_name: "my_plugin.dll".into(),
    metadata: PluginMetadata::default(),
})?;
std::fs::write("MyPlugin.ACT.dll", shim)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

두 결과물만 플러그인 전용 폴더에 함께 둡니다.

```text
MyPlugin/
  MyPlugin.ACT.dll
  my_plugin.dll
```

ACT에서 FFXIV Parsing Plugin을 먼저 활성화한 뒤 `MyPlugin.ACT.dll`을 추가합니다.
SDK의 Common DLL은 빌드 입력일 뿐 결과물 옆에 복사하지 않습니다. 이벤트
콜백은 Rust에서 직렬화되며 `Event<'_>`의 빌린 데이터와 `Repository<'_>`는
콜백 밖으로 보관할 수 없습니다. Repository가 반환하는 바이트는
`bytes::Bytes`, 모델과 문자열은 소유 값입니다. Rust DLL의 대상 아키텍처는
실행 중인 ACT 프로세스와 같아야 합니다.

실행 가능한 최소 예제는 `examples/ffxiv-plugin`에 있습니다. 지역 변경을
`%TEMP%\FfxivActNativeExample.log`에 기록하며, build script가 managed shim도 함께
생성합니다. 빌드에는 ACT나 FFXIV ACT Plugin SDK 설치가 필요하지 않습니다.

```powershell
cargo build --manifest-path examples\ffxiv-plugin\Cargo.toml
```

`examples\ffxiv-plugin\target\debug`의 `FfxivActNativeExample.ACT.dll`과
`ffxiv_plugin.dll`을 같은 플러그인 폴더에 복사한 뒤 ACT에는 managed DLL을
추가합니다. 릴리스 빌드는 `--release`를 추가하고 `target\release`를 사용합니다.

선택적 실제 SDK 테스트:

```powershell
$env:FFXIV_ACT_NATIVE_ACT_EXE = 'C:\path\Advanced Combat Tracker.exe'
$env:FFXIV_ACT_NATIVE_COMMON_DLL = 'C:\path\FFXIV_ACT_Plugin.Common.dll'
cargo test optional_real_sdk_generation
```
