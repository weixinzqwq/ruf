"""Exercise the embedded MyPro tools page in offline Chromium using synthetic IPC."""
from pathlib import Path
import json
from urllib.parse import urlsplit

from playwright.sync_api import expect, sync_playwright


PANEL = Path(__file__).resolve().parents[1] / "optimization/engine/desktop/src/mypro_panel/index.html"
PANEL_URL = "http://mypro.localhost/index.html"
ASSETS = PANEL.parents[2] / "assets/battle-assistant"
SUMMONER = {"id": "summoner", "name": "召唤师",
            "url": "activityres/activity250516/newzhaohuanshi/newzhaohuanshi_code",
            "cls": "mmo.activity250516.newzhaohuanshi.panel.NZHS_MainPl"}


class Fixture:
    def __init__(self):
        self.session = "fixture-one"
        self.retaining = False
        self.intercepting = False
        self.packet_rows = []
        self.capturing = False
        self.capture_rows = []
        self.morph = {"revision": 4, "stage": "ready", "enabled": False, "enemy": False,
                      "rules": [{"remark": "初始测试", "originalId": "5336", "backpackId": "6044",
                                 "fightId": "/", "spskiId": "/"}]}
        self.battle = None
        self.display = {"battleInputNormalFps": True}
        self.auto = False
        self.background = False
        self.theme = 0
        self.plans = []
        self.session_events = 0
        self.calls = []
        self.errors = []
        self.last_request = 0

    def request(self, message):
        try:
            return self.reply(message)
        except Exception as exc:
            self.errors.append(str(exc))
            raise

    def reply(self, message):
        command = message["command"]
        args = message["payload"]
        self.calls.append((command, args, message["sessionId"]))
        assert message["protocolVersion"] == 1 and message["bridgeGeneration"] == 3
        request = int(message["requestId"].removeprefix("ui-"))
        assert request > self.last_request
        self.last_request = request
        if command == "app.snapshot":
            assert message["sessionId"] is None
            capabilities = ["packets.page", "packets.retain.set", "packets.export", "packets.intercept.set",
                            "packets.clear", "battle.snapshot", "battle.details", "panel_capture.toggle",
                            "panel_capture.clear", "panel_capture.list", "morph.rules.read",
                            "morph.rules.replace", "morph.restore"]
            capabilities += ["settings.read", "settings.patch", "background.read", "background.choose",
                             "background.clear", "battle.auto.read", "battle.auto.set"]
            capabilities += ["workshop.validate", "workshop.readOwnBag", "game.panels", "game.panel.open", "theme.read", "theme.apply",
                             "lineups.read", "lineups.refresh", "lineups.save", "lineups.restore"]
            return {"hasMovie": True, "playing": True, "sessionId": self.session,
                    "viewport": {"width": 1200, "height": 700}, "coreBuild": "offline",
                    "capabilities": {key: {"available": True} for key in capabilities}}
        assert message["sessionId"] == self.session
        if command == "panel_capture.toggle":
            self.capturing = args["enabled"]
        if command == "panel_capture.clear":
            self.capture_rows = []
        if command.startswith("panel_capture."):
            return {"enabled": self.capturing, "rows": self.capture_rows, "dropped": 0}
        if command in ("morph.rules.replace", "morph.restore"):
            assert args["expectedRevision"] == self.morph["revision"]
            self.morph["revision"] += 1
            if command == "morph.restore":
                self.morph["enabled"] = False
            else:
                self.morph.update({k: args[k] for k in ("enabled", "enemy", "rules")})
        if command.startswith("morph."):
            return self.morph.copy()
        if command == "workshop.validate":
            if args["text"] == '#send={"id":13,"cmd":"1212","param":{}}|':
                return {"steps": [{"line": 1, "kind": "message", "id": 13, "cmd": "1212", "fields": 0}], "canReadOwnBag": True}
            if args["text"].lstrip().startswith("#send="):
                item = json.loads(args["text"].strip().removesuffix("|").removeprefix("#send="))
                return {"steps": [{"line": 1, "kind": "message", "id": item["id"],
                                   "cmd": item["cmd"], "fields": 0}]}
            assert args["text"] == "#time = 0.5 | #wait"
            return {"steps": [{"line": 1, "kind": "delay", "seconds": 0.5}, {"line": 1, "kind": "wait"}]}
        if command == "game.panels":
            return {"panels": [SUMMONER]}
        if command == "game.panel.open":
            assert args in ({"id": "summoner"}, {k: SUMMONER[k] for k in ("url", "cls")})
            return {"status": "requested"}
        if command.startswith("theme."):
            if command == "theme.apply":
                self.theme = args["themeId"]
            return {"status": "WaitingBattleEnd" if self.theme else "Applied", "themeId": self.theme, "appliedThemeId": 0}
        if command.startswith("lineups.") or command == "workshop.readOwnBag":
            if command == "lineups.save":
                self.plans.append(args["name"])
            if command == "lineups.restore":
                assert args == {"name": "双槽", "section": "order"}
            return {"plans": self.plans, "canSave": True, "sections": ["order"],
                    "stage": "ready", "message": "顺序已回读" if command.endswith("restore") else "",
                    "current": {"pets": [{"petId": 80, "raceId": 17, "name": "背包亚比"}]}}
        if command in ("settings.read", "settings.patch"):
            if command == "settings.patch":
                self.display.update(args["values"])
            return {"stage": "ready", "revision": len(self.display), "values": self.display}
        if command.startswith("background."):
            if command != "background.read":
                self.background = command == "background.choose"
            return {"stage": "ready", "configured": self.background}
        if command.startswith("battle.auto."):
            if command == "battle.auto.set":
                self.auto = args["enabled"]
            return {"status": "ready" if command.endswith("read") else "applied", "enabled": self.auto}
        if command == "packets.intercept.set":
            self.intercepting = args["enabled"]
            return {"enabled": self.intercepting}
        if command == "packets.clear":
            self.packet_rows = []
            return {"cleared": True}
        if command == "packets.page":
            return {
                "rows": self.packet_rows,
                "oldestSeq": 1, "newestSeq": len(self.packet_rows), "nextAfter": None,
                "interceptEnabled": self.intercepting,
                "gap": False, "droppedRows": 0, "droppedChunks": 0, "gaps": 0, "retaining": self.retaining,
                "logFile": "logs/fixture.jsonl" if self.retaining else None, "logError": None, "droppedLogRows": 0,
            }
        if command == "packets.retain.set":
            self.retaining = args["enabled"]
            return {"requested": self.retaining}
        if command == "packets.export":
            item = next(row for row in self.packet_rows if row["seq"] == args["seq"])
            return {"seq": item["seq"], "text": "#send=" + json.dumps(item["data"]) + "|"}
        if command == "battle.snapshot":
            details = self.battle or {"state": "unknown", "battleUniqueId": None,
                                      "selfPmmId": None, "lastCommand": None, "enemy": None}
            return {"state": details["state"], "battleId": 1, "switches": 0, "source": "offline",
                    "updatedMs": 1800000000000, "details": details}
        raise AssertionError(f"Unexpected panel command: {command}")


BRIDGE = """
window.myproGeneration = 3;
Object.defineProperty(navigator, "clipboard", {value: {
  async writeText(text) { window.fixtureClipboard = text; }
}});
window.ipc = { postMessage(raw) {
  const msg = JSON.parse(raw);
  window.fixtureRequest(msg).then(result => {
    const deliver = () => window.myproReceive({protocolVersion: 1, bridgeGeneration: 3,
      kind: "response", requestId: msg.requestId, ok: true, result});
    if (window.holdNext === msg.command) {
      window.holdNext = null;
      window.heldReply = deliver;
    } else {
      deliver();
    }
  });
}};
"""


def switch_session(page, fixture, session):
    fixture.session = session
    fixture.session_events += 1
    fixture.retaining = fixture.intercepting = fixture.capturing = False
    fixture.packet_rows = []
    fixture.capture_rows = []
    page.evaluate("""([seq,id]) => window.myproReceive({protocolVersion:1, kind:'event', bridgeGeneration:3,
       seq, event:'session.changed', sessionId:id, payload:{sessionId:id}})""", [fixture.session_events, session])
    expect(page.locator("#session")).to_have_text(session)


def packets(page, fixture):
    page.locator("[data-tab=messages]").click()
    page.locator("#messages-tab-capture").click()
    toggle = page.locator("#packet-intercept")
    toggle.click()
    expect(toggle).to_have_text("停止拦截")
    assert fixture.intercepting
    fixture.packet_rows = [
        {"seq": 1, "direction": "send", "data": {"cmd": "checkNickName", "id": 1, "param": None},
         "exportable": True, "decoded": True},
        {"seq": 2, "direction": "recv", "data": {"cmd": "not-a-visible-recv"}, "exportable": False},
    ]
    rows = page.locator("#packet-rows")
    row = rows.get_by_role("row").filter(has_text="checkNickName")
    expect(row).to_contain_text("send")
    expect(rows).not_to_contain_text("not-a-visible-recv")
    row.get_by_role("checkbox").check()
    page.locator("#packet-copy").click()
    page.wait_for_function('typeof window.fixtureClipboard === "string"')
    assert json.loads(page.evaluate("window.fixtureClipboard")) == fixture.packet_rows[0]["data"]
    page.locator("#packet-insert").click()
    page.locator("#messages-tab-send").click()
    text = page.locator("#workshop-text")
    page.wait_for_function('document.querySelector("#workshop-text").value.includes("checkNickName")')
    page.locator("#workshop-run").click()
    expect(page.locator("#workshop-steps")).to_contain_text("checkNickName")
    page.locator("#messages-tab-capture").click()
    page.locator("#packet-retain").click()
    expect(page.locator("#packet-status")).to_contain_text("fixture.jsonl")
    assert fixture.retaining
    toggle.click()
    expect(toggle).to_have_text("开始拦截")
    assert not fixture.intercepting
    page.locator("#packet-clear").click()
    expect(rows).not_to_contain_text("checkNickName")
    assert fixture.retaining
    page.locator("#packet-clear-text").click()
    expect(text).to_have_value("")

    toggle.click()
    expect(toggle).to_have_text("停止拦截")
    fixture.packet_rows = [
        {"seq": 3, "direction": "send", "data": {"cmd": "checkNickName", "id": 1, "param": None},
         "exportable": True, "decoded": True},
    ]
    row.get_by_role("checkbox").check()
    page.evaluate("window.holdNext = 'packets.export'")
    page.locator("#packet-insert").click()
    page.wait_for_function("typeof window.heldReply === 'function'")
    switch_session(page, fixture, "fixture-two")
    page.evaluate("window.heldReply()")
    expect(rows).not_to_contain_text("checkNickName")
    expect(text).to_have_value("")


def capture(page, fixture):
    page.locator("[data-tab=capture]").click()
    page.locator("#record").click()
    expect(page.locator("#record")).to_have_text("暂停采集")
    assert fixture.capturing
    fixture.capture_rows = [{"seq": 1, "timeMs": 1800000000000, **SUMMONER}]
    row = page.locator("#capture-rows").get_by_role("row").filter(has_text=SUMMONER["url"])
    expect(row).to_contain_text(SUMMONER["cls"])
    row.click()
    page.locator("#capture-copy").click()
    page.wait_for_function('typeof window.fixtureClipboard === "string"')
    assert page.evaluate("window.fixtureClipboard") == f'{SUMMONER["url"]}\t{SUMMONER["cls"]}'
    page.locator("#capture-fill").click()
    expect(page.locator("#game-panel-url")).to_have_value(SUMMONER["url"])
    expect(page.locator("#game-panel-cls")).to_have_value(SUMMONER["cls"])
    page.locator("#game-panel-open").click()
    expect(page.locator("#game-panel-status")).to_contain_text("已受理")
    page.locator("[data-tab=capture]").click()
    page.locator("#record").click()
    expect(page.locator("#record")).to_have_text("开始采集")
    assert not fixture.capturing
    expect(row).to_be_visible()
    page.locator("#capture-clear").click()
    expect(row).to_have_count(0)


def morph(page, fixture):
    page.locator("[data-tab=morph]").click()
    expect(page.get_by_role("heading", name="应用状态")).to_have_count(0)
    original = page.locator("#morph-rows").get_by_role("row").filter(has_text="初始测试")
    expect(original).to_contain_text("6044")
    search = page.locator("#morph-search")
    search.fill("未知内容")
    expect(original).to_have_count(0)
    search.fill("6044")
    expect(original).to_be_visible()
    search.clear()
    page.locator("#m-remark").fill("新规则")
    page.locator("#m-originalId").fill("5000")
    page.locator("#m-backpackId").fill("6044")
    page.locator("#morph-add").click()
    expect(page.locator("#morph-rows")).to_contain_text("新规则")
    page.locator("#morph-enabled").check()
    page.locator("#morph-save").click()
    expect(page.locator("#morph-save")).to_be_disabled()
    assert fixture.morph["enabled"] and len(fixture.morph["rules"]) == 2
    page.locator("#morph-restore").click()
    expect(page.locator("#morph-enabled")).not_to_be_checked()
    assert not fixture.morph["enabled"] and len(fixture.morph["rules"]) == 2
    page.locator("#morph-import").click()
    page.locator("#morph-json").fill('{"skins":[{"originalId":5099,"fightId":6000,"remark":"旧配置"}]}')
    page.locator("#morph-import-apply").click()
    imported = page.locator("#morph-rows").get_by_role("row").filter(has_text="旧配置")
    expect(imported).to_contain_text("6000")
    imported.click()
    page.locator("#morph-remove").click()
    expect(imported).to_have_count(0)


def opponent(owner, name, hp):
    ability_names = ["生命", "攻击", "防御", "特攻", "特防", "速度"]
    pet = {
        "uqi": 88, "slotId": 1, "raceId": 124891, "viewId": 124891, "name": name, "nickname": name,
        "level": 100, "currentHp": hp, "maximumHp": 701,
        "abilities": [{"field": f"ea{n}", "name": stat, "total": 210 + n, "segments": [200, 10 + n]}
                      for n, stat in enumerate(ability_names)],
        "skills": [{"id": 88, "name": "离线招式", "pp": 13, "cd": 2}],
        "loadout": {"equipment": {"state": "known", "items": [{"id": 10227, "instanceId": 531,
                                                               "details": "夹具装备详情"}, {"id": 0}]},
                    "soul-card": {"state": "empty", "items": [{"id": 0}, {"id": 0}]},
                    "soul-artifact": {"state": "invalid", "items": []},
                    "crystal-key": {"state": "notProvided", "items": []}},
    }
    return {"state": "active", "battleUniqueId": f"fixture-{owner}", "selfPmmId": 700,
            "lastCommand": "2401", "enemy": {"pmmId": owner, "pets": [pet]}}


def battle(page, fixture):
    enemy = page.locator("#enemy-pets")
    fixture.battle = opponent(-101, "离线敌宠甲", 234)
    page.locator("[data-tab=battle]").click()
    page.locator("#battle-attributes").click()
    for text in ("离线敌宠甲", "HP 234 / 701", "生命 210", "攻击 211", "防御 212",
                 "特攻 213", "特防 214", "速度 215", "离线招式 · PP 13 · CD 2"):
        expect(enemy).to_contain_text(text)
    page.wait_for_function("""() => document.querySelector('#enemy-pets img[src$="/pet/124891.png"]')?.naturalWidth > 0""")

    page.locator("#battle-config").click()
    for category in ("装备", "魂卡"):
        group = enemy.locator(".row").filter(has=page.get_by_text(category, exact=True))
        expect(group.get_by_text("空", exact=True)).to_have_count(1)
    expect(enemy).not_to_contain_text("#0")
    for text in ("装备", "魂卡", "魂器", "晶钥", "鹿鸣仙风王者战剑LV3 #10227", "空", "无法解析", "尚未收到"):
        expect(enemy).to_contain_text(text)
    page.wait_for_function("""() => document.querySelector('#enemy-pets img[src$="/equipment/10227.png"]')?.naturalWidth > 0""")
    expect(enemy).not_to_contain_text("离线招式")

    fixture.battle = opponent(701, "离线敌宠乙", 413)
    switch_session(page, fixture, "fixture-pvp")
    page.locator("#battle-attributes").click()
    expect(enemy).to_contain_text("离线敌宠乙")
    expect(enemy).to_contain_text("413")
    expect(enemy).not_to_contain_text("离线敌宠甲")

    fixture.battle = None
    switch_session(page, fixture, "fixture-spectator")
    expect(enemy).not_to_contain_text("离线敌宠乙")
    page.locator("#battle-config").click()
    expect(enemy).to_have_text("等待可识别的敌方阵容")


def local_file(route):
    url = urlsplit(route.request.url)
    if route.request.url == PANEL_URL:
        return route.fulfill(path=PANEL, content_type="text/html; charset=utf-8")
    if url.netloc == "mypro.localhost":
        if url.path == "/battle-assets/names.json":
            return route.fulfill(path=ASSETS / "names.json", content_type="application/json")
        if url.path in {"/battle-assets/pet/124891.png", "/battle-assets/equipment/10227.png"}:
            return route.fulfill(path=ASSETS / url.path.removeprefix("/battle-assets/"), content_type="image/png")
    return route.abort()


def presentation(page, fixture):
    page.locator("[data-tab=settings]").click()
    damage = page.locator("#setting-hideBattleDamage")
    healing = page.locator("#setting-hideBattleHealing")
    normal_fps = page.get_by_role("checkbox", name="选技能时恢复原版帧率（防回合倒计时过快）")
    expect(normal_fps).to_be_checked()
    normal_fps.uncheck()
    expect(normal_fps).not_to_be_checked()
    expect(normal_fps).to_be_enabled()
    assert fixture.display["battleInputNormalFps"] is False
    damage.check()
    expect(damage).to_be_checked()
    expect(healing).not_to_be_checked()
    healing.check()
    page.locator("#setting-skipBattleAnimations").check()
    expect(damage).to_be_checked()
    auto = page.locator("#native-auto")
    auto.check()
    expect(page.locator("#auto-status")).to_contain_text("已开启")
    page.locator("#background-choose").click()
    expect(page.locator("#background-status")).to_contain_text("960×560")
    page.locator("#background-clear").click()
    expect(page.locator("#background-status")).to_have_text("使用游戏背景")
    assert fixture.display == {"battleInputNormalFps": False, "hideBattleDamage": True,
                               "hideBattleHealing": True, "skipBattleAnimations": True}


def main():
    fixture = Fixture()
    with sync_playwright() as runtime:
        browser = runtime.chromium.launch(channel="msedge", headless=True,
                                          args=["--disable-background-networking", "--no-proxy-server", "--disable-gpu"])
        context = browser.new_context(offline=True, locale="zh-CN", viewport={"width": 680, "height": 580})
        context.route("**/*", local_file)
        context.expose_function("fixtureRequest", fixture.request)
        context.add_init_script(BRIDGE)
        page = context.new_page()
        page.on("pageerror", lambda error: fixture.errors.append(str(error)))
        page.goto(PANEL_URL)
        expect(page.locator("#session")).to_have_text("fixture-one")
        capture(page, fixture)
        morph(page, fixture)
        packets(page, fixture)
        battle(page, fixture)
        presentation(page, fixture)
        page.locator("#theme-choice").select_option("10")
        page.locator("#theme-apply").click()
        expect(page.locator("#theme-status")).to_contain_text("待战斗结束")
        page.locator("[data-tab=messages]").click()
        page.locator("#messages-tab-send").click()
        page.get_by_role("button", name="召唤师").click()
        expect(page.locator("#game-panel-status")).to_contain_text("已受理")
        page.locator("#workshop-text").fill("#time = 0.5 | #wait")
        page.locator("#workshop-run").click()
        expect(page.locator("#workshop-status")).to_have_text("等待模拟退场")
        page.locator("#workshop-next").click()
        expect(page.locator("#workshop-status")).to_have_text("演练完成")
        expect(page.locator("#own-bag-action")).to_be_hidden()
        page.locator("#workshop-text").fill('#send={"id":13,"cmd":"1212","param":{}}|')
        page.locator("#workshop-run").click()
        page.locator("#own-bag-read").click()
        expect(page.locator("#own-bag-status")).to_contain_text("已核对当前背包，共 1 只")
        page.locator("[data-tab=lineups]").click()
        expect(page.locator("#lineup-pets")).to_contain_text("个体 80")
        page.locator("#lineup-read").click()
        expect(page.locator("#lineup-read")).to_be_enabled()
        assert any(cmd == "lineups.refresh" for cmd, _, _ in fixture.calls)
        page.locator("#lineup-name").fill("双槽")
        page.locator("#lineup-save").click()
        page.locator("#lineup-restore").click()
        expect(page.locator("#lineup-status")).to_have_text("顺序已回读")
        assert not fixture.errors, fixture.errors
        browser.close()
    print("PASS panel capture, morph rules, message tabs, enemy slots, themes and lineups")


if __name__ == "__main__":
    main()
