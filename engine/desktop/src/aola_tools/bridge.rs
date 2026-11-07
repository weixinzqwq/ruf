//! Minimal original-session send adapter. The game owns encryption and sequences.
use ruffle_core::backend::navigator::{ErrorResponse, NavigationMethod, OwnedFuture, Request, SuccessResponse};
use ruffle_core::external::Value;
use ruffle_core::loader::Error;
use ruffle_core::swf::Encoding;
use ruffle_core::Player;
use std::borrow::Cow;
use url::Url;

pub const START_PATH: &str = "/play/start~2107300001.swf";
pub const BYTES: &[u8] = include_bytes!("../../../../source/launcher-bridge/variants/start-launcher.fws.swf");

pub fn accepts(request: &Request, url: &Url) -> bool {
    request.method() == NavigationMethod::Get && request.body().is_none()
        && matches!(url.scheme(), "http" | "https")
        && url.host_str() == Some("aola.100bt.com") && url.port().is_none()
        && url.username().is_empty() && url.password().is_none()
        && url.path() == START_PATH
}

pub fn invoke(player: &mut Player, request_id: &str, extension_id: u16, command: &str, parameters: &serde_json::Value) -> String {
    let Some(extension) = extension_name(extension_id) else { return "UnknownExtension".into() };
    // The reference host routes native auto mode through its battle bridge.
    // Do not claim that a raw 1409 reproduces that stateful operation in phase 1.
    if extension_id == 13 && command == "1409" { return "BattleBridgeRequired".into(); }
    if !parameters.is_object() { return "InvalidParameters".into(); }
    match player.call_internal_interface("aolaSendCommand", [
        Value::String(request_id.into()), Value::String(extension.into()),
        Value::String(command.into()), Value::String(parameters.to_string()),
    ]) {
        Value::String(status) => status,
        _ => "BridgeNotReady".into(),
    }
}

pub fn ready(player: &mut Player) -> bool {
    matches!(player.call_internal_interface("aolaBridgeReady", []), Value::String(value) if value == "AolaLauncherBridge1")
}

pub fn extension_name(id: u16) -> Option<&'static str> {
    match id {
        // Kept in sync with the source-backed map in the minimal SWF bridge.
        0 => Some("StartupExtension"),
        1 => Some("UserExtension"),
        2 => Some("TeamExtension"),
        3 => Some("BuddyManagerExtension"),
        4 => Some("ScoreManager"),
        5 => Some("Log"),
        6 => Some("SurveyExtension"),
        7 => Some("PlayBookExtension"),
        8 => Some("OneOffExtension"),
        9 => Some("MemberExtension"),
        10 => Some("GameRealTimeRankManager"),
        11 => Some("GameMessageList"),
        12 => Some("UserDisplayExtension"),
        13 => Some("PocketMonsterExtension"),
        14 => Some("PetFightExtension"),
        15 => Some("PetExtension"),
        16 => Some("PetExtension2"),
        17 => Some("PetEggExtension"),
        18 => Some("AntiAddictionExtension"),
        19 => Some("TowerExtension"),
        20 => Some("SynthesizeExtension"),
        21 => Some("MaterialExtension"),
        22 => Some("RobotExtension"),
        23 => Some("MatchExtension"),
        24 => Some("FightRingExtension"),
        25 => Some("RobMedalArenaExtension"),
        26 => Some("HouseExtension"),
        27 => Some("UserFurntitureInfoExtension"),
        28 => Some("ShootExtension"),
        29 => Some("TaskExtension"),
        30 => Some("DailyTaskExtension"),
        31 => Some("GiftBagExtension"),
        32 => Some("AvatarExtension"),
        33 => Some("FollowExtension"),
        34 => Some("SundriesExtension"),
        35 => Some("FBExtension"),
        36 => Some("AvocationExtension"),
        37 => Some("MidautumnGameExtension"),
        38 => Some("MoveStateExtension"),
        39 => Some("UserStateExtension"),
        40 => Some("AchievementExtension"),
        41 => Some("NewAreaExtension"),
        42 => Some("HolidayExtension"),
        43 => Some("MailBoxExtension"),
        44 => Some("PTowerExtension"),
        45 => Some("PMDailyCollectExtension"),
        46 => Some("VipNickNameExtension"),
        47 => Some("VipTowerExtension"),
        48 => Some("PlantExtension"),
        49 => Some("PlantItemExtension"),
        50 => Some("ConnectTestExtension"),
        52 => Some("WisdomStarExtension"),
        53 => Some("DonateExtension"),
        54 => Some("BalloonExtension"),
        55 => Some("UserGameExtension"),
        56 => Some("PlaneChessGameManager"),
        57 => Some("PeriodicActiveExtension"),
        58 => Some("MagicCubicExtension"),
        59 => Some("WarTeamExtension"),
        60 => Some("WTBeatBossExtension"),
        61 => Some("HouseOutSideExtension"),
        62 => Some("WarTeamTaskExtension"),
        63 => Some("PiracyTowerExtension"),
        64 => Some("MentalHallExtension"),
        65 => Some("ZooExtension"),
        66 => Some("StateExtension"),
        67 => Some("HotSpringExtension"),
        68 => Some("MSExtension"),
        69 => Some("LabExtension"),
        70 => Some("KOFExtension"),
        71 => Some("FieldGameExtension"),
        72 => Some("HbhBeatBossExtension"),
        73 => Some("TeamTowerExtension"),
        74 => Some("LuandouExtension"),
        75 => Some("Sundries2ndExtension"),
        76 => Some("GalaxyExtension"),
        77 => Some("PetEquipExtension"),
        78 => Some("BattleGodExtension"),
        79 => Some("CorpsExtension"),
        81 => Some("SpeedTestExtension"),
        82 => Some("LightWingExtension"),
        83 => Some("XSystemExtension"),
        84 => Some("StarBossExtension"),
        85 => Some("FiveYearPKExtension"),
        86 => Some("KaiSaExtension"),
        87 => Some("BurningCrusadeExtension"),
        99 => Some("TrainingExtension"),
        _ => None,
    }
}

struct BridgeResponse { url: String, remaining: Option<Vec<u8>> }

pub fn response(url: String) -> OwnedFuture<Box<dyn SuccessResponse>, ErrorResponse> {
    Box::pin(async move {
        Ok(Box::new(BridgeResponse { url, remaining: Some(BYTES.to_vec()) }) as Box<dyn SuccessResponse>)
    })
}

impl SuccessResponse for BridgeResponse {
    fn url(&self) -> Cow<'_, str> { Cow::Borrowed(&self.url) }
    fn set_url(&mut self, url: String) { self.url = url; }
    fn body(self: Box<Self>) -> OwnedFuture<Vec<u8>, Error> {
        Box::pin(async move { Ok(self.remaining.unwrap_or_default()) })
    }
    fn text_encoding(&self) -> Option<&'static Encoding> { None }
    fn status(&self) -> u16 { 200 }
    fn redirected(&self) -> bool { false }
    fn next_chunk(&mut self) -> OwnedFuture<Option<Vec<u8>>, Error> {
        let bytes = self.remaining.take();
        Box::pin(async move { Ok(bytes) })
    }
    fn expected_length(&self) -> Result<Option<u64>, Error> { Ok(Some(BYTES.len() as u64)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aola_bridge_forwards_arguments_and_distinguishes_offline_callback_results() {
        use ruffle_core::context::UpdateContext;
        use ruffle_core::external::ExternalInterfaceProvider;
        use ruffle_core::{PlayerBuilder, tag_utils::SwfMovieData};
        use std::sync::{Arc, Mutex};
        type Calls = Arc<Mutex<Vec<(String, Vec<Value>)>>>;
        struct Provider(Calls);
        impl ExternalInterfaceProvider for Provider {
            fn call_method(&self, _: &mut UpdateContext<'_>, name: &str, args: &[Value]) -> Value {
                self.0.lock().unwrap().push((name.into(), args.to_vec()));
                Value::Undefined
            }
            fn on_callback_available(&self, _: &str) {}
            fn get_id(&self) -> Option<String> { Some("offline-mock-sender".into()) }
        }
        let bytes = include_bytes!("../../../../source/launcher-bridge/fixtures/start-with-mock.swf");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let movie = SwfMovieData::from_data(bytes, format!("https://aola.100bt.com{START_PATH}"), None, None).unwrap();
        let player = PlayerBuilder::new().with_movie(movie)
            .with_external_interface(Box::new(Provider(calls.clone()))).build();
        let mut player = player.lock().unwrap();
        player.run_frame();
        assert!(ready(&mut player));
        for (command, status, code) in [
            ("fixture-success", "ServerSucceeded", "1"),
            ("fixture-rejected", "ServerRejected", "0"),
            ("fixture-unknown", "ServerResponse", ""),
        ] {
            calls.lock().unwrap().clear();
            let request_id = "00000000000000000000000000000001";
            assert_eq!(invoke(&mut player, request_id, 42, command, &serde_json::json!({"text":"中文|quoted",":ext_seq;":123})), "SubmittedToFlash");
            let calls = calls.lock().unwrap();
            let sent = &calls.iter().find(|(name,_)| name == "fixtureSend").unwrap().1;
            assert!(matches!(&sent[0], Value::String(value) if value == "HolidayExtension"));
            assert!(matches!(&sent[1], Value::String(value) if value == command));
            let Value::String(parameters) = &sent[2] else { panic!("JSON parameters") };
            assert_eq!(serde_json::from_str::<serde_json::Value>(parameters).unwrap(), serde_json::json!({"text":"中文|quoted"}));
            assert!(matches!(&sent[3], Value::Bool(false)));
            let completed = &calls.iter().find(|(name,_)| name == "aolaCommandCompleted").unwrap().1;
            assert!(matches!(&completed[0], Value::String(value) if value == request_id));
            assert!(matches!(&completed[1], Value::String(value) if value == status));
            assert!(matches!(&completed[2], Value::String(value) if value == code));
        }
    }
    #[test]
    fn aola_bridge_loads_offline_registers_and_rejects_without_game_sender() {
        use ruffle_core::context::UpdateContext;
        use ruffle_core::external::ExternalInterfaceProvider;
        use ruffle_core::{PlayerBuilder, tag_utils::SwfMovieData};
        use std::sync::{Arc, Mutex};
        struct Provider(Arc<Mutex<Vec<String>>>);
        impl ExternalInterfaceProvider for Provider {
            fn call_method(&self, _: &mut UpdateContext<'_>, _: &str, _: &[Value]) -> Value { Value::Undefined }
            fn on_callback_available(&self, name: &str) { self.0.lock().unwrap().push(name.into()); }
            fn get_id(&self) -> Option<String> { Some("offline-launcher-test".into()) }
        }
        for (bytes, patched) in [
            (include_bytes!("../../../../source/launcher-bridge/source/start-original.fws.swf").as_slice(), false),
            (BYTES, true),
        ] {
            let callbacks = Arc::new(Mutex::new(Vec::new()));
            let movie = SwfMovieData::from_data(bytes, format!("https://aola.100bt.com{START_PATH}"), None, None).unwrap();
            // The default null navigator cannot issue network requests.
            let player = PlayerBuilder::new().with_movie(movie)
                .with_external_interface(Box::new(Provider(callbacks.clone()))).build();
            let mut player = player.lock().unwrap();
            player.run_frame();
            assert_eq!(callbacks.lock().unwrap().contains(&"aolaSendCommand".into()), patched);
            if patched {
                assert!(!ready(&mut player));
                assert_eq!(invoke(&mut player, "00000000000000000000000000000001", 42, "example", &serde_json::json!({})), "BridgeNotReady");
                assert_eq!(invoke(&mut player, "00000000000000000000000000000002", 51, "example", &serde_json::json!({})), "UnknownExtension");
            }
        }
    }
    #[test]
    fn aola_bridge_only_replaces_supported_start_resource() {
        let address = format!("https://aola.100bt.com{START_PATH}");
        assert!(accepts(&Request::get(address.clone()), &Url::parse(&address).unwrap()));
        for invalid in [address.replace("100bt.com", "100bt.com.invalid"), address.replace("2107300001", "999"), address.replace("https:", "file:")] {
            assert!(!accepts(&Request::get(invalid.clone()), &Url::parse(&invalid).unwrap()));
        }
        assert!(!accepts(&Request::post(address.clone(), None), &Url::parse(&address).unwrap()));
        assert_eq!(extension_name(13), Some("PocketMonsterExtension"));
        assert_eq!(extension_name(99), Some("TrainingExtension"));
        assert_eq!(extension_name(51), None);
        assert_eq!(&BYTES[..3], b"FWS");
    }
    #[tokio::test]
    async fn aola_bridge_response_keeps_official_url_and_returns_bytes_once() {
        let url = format!("https://aola.100bt.com{START_PATH}");
        let mut value = match response(url.clone()).await { Ok(value) => value, Err(_) => panic!("local response") };
        assert_eq!(value.url(), url);
        assert_eq!(value.next_chunk().await.unwrap().unwrap(), BYTES);
        assert!(value.next_chunk().await.unwrap().is_none());
    }
}
