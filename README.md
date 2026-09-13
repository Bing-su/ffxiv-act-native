# act-bridge-gen

Rust `cdylib`으로 FFXIV ACT 플러그인을 작성할 수 있도록 AnyCPU .NET Framework
shim을 생성합니다. 생성기는 `Advanced Combat Tracker.exe`와 공식 SDK의
`FFXIV_ACT_Plugin.Common.dll`만 읽으며 `FFXIV_ACT_Plugin.dll`은 입력이나 참조로
사용하지 않습니다.

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
act-bridge-gen = "0.1"
```

```rust
use act_bridge_gen::{
    Event, Plugin, PluginResult, Repository, SubscriptionSet, export_plugin,
};

struct MyPlugin;

impl Plugin for MyPlugin {
    fn init(repository: Repository<'_>) -> PluginResult<(Self, SubscriptionSet)> {
        let _player_id = repository.current_player_id()?;
        Ok((Self, SubscriptionSet::ZONE_CHANGED | SubscriptionSet::LOG_LINE))
    }

    fn on_event(&mut self, _: Repository<'_>, event: Event<'_>) -> PluginResult<()> {
        eprintln!("{event:?}");
        Ok(())
    }

    fn shutdown(&mut self, _: Repository<'_>) -> PluginResult<()> { Ok(()) }
}

export_plugin!(MyPlugin);
```

호출자의 생성 프로그램에서 shim을 저장합니다. 별도 CLI나 소비자 `build.rs`
통합은 없습니다.

```rust,no_run
use act_bridge_gen::{PluginConfig, generate};

let act = std::fs::read(r"C:\Program Files\Advanced Combat Tracker\Advanced Combat Tracker.exe")?;
let common = std::fs::read(r"sdk\FFXIV_ACT_Plugin.Common.dll")?;
let shim = generate(&act, &common, &PluginConfig {
    assembly_name: "MyPlugin.ACT".into(),
    native_dll_name: "my_plugin.dll".into(),
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

선택적 실제 SDK 테스트:

```powershell
$env:ACT_BRIDGE_ACT_EXE = 'C:\path\Advanced Combat Tracker.exe'
$env:ACT_BRIDGE_COMMON_DLL = 'C:\path\FFXIV_ACT_Plugin.Common.dll'
cargo test optional_real_sdk_generation
```

라이선스는 `GPL-3.0-or-later`입니다.
