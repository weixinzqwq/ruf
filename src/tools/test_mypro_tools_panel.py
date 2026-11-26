"""Exercise the embedded MyPro tools page in offline Chromium using synthetic IPC."""
from pathlib import Path
from urllib.parse import urlsplit

from playwright.sync_api import expect, sync_playwright


PANEL = Path(__file__).resolve().parents[1] / "optimization/engine/desktop/src/mypro_panel/index.html"
PANEL_URL = "http://mypro.localhost/index.html"
ASSETS = PANEL.parents[2] / "assets/battle-assistant"


class Fixture:
    def __init__(self):
        self.session = "fixture-one"
        self.retaining = False
        self.latest_summary = "初始记录"
        self.battle = None
        self.display = {}
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
            capabilities = ["packets.page", "packets.retain.set", "packets.export", "battle.snapshot", "battle.details"]
            capabilities += ["settings.read", "settings.patch", "background.read", "background.choose",
                             "background.clear", "battle.auto.read", "battle.auto.set"]
            capabilities += ["workshop.validate", "workshop.readOwnBag", "game.panels", "game.panel.open", "theme.read", "theme.apply",
                             "lineups.read", "lineups.refresh", "lineups.save", "lineups.restore"]
            return {"hasMovie": True, "playing": True, "sessionId": self.session,
                    "viewport": {"width": 1200, "height": 700}, "coreBuild": "offline",
                    "capabilities": {key: {"available": True} for key in capabilities}}
        assert message["sessionId"] == self.session
        if command == "workshop.validate":
            if args["text"] == '#send={"id":13,"cmd":"1212","param":{}}|':
                return {"steps": [{"line": 1, "kind": "message", "id": 13, "cmd": "1212", "fields": 0}], "canReadOwnBag": True}
            assert args["text"] == "#time = 0.5 | #wait"
            return {"steps": [{"line": 1, "kind": "delay", "seconds": 0.5}, {"line": 1, "kind": "wait"}]}
        if command == "game.panels":
            return {"panels": [{"id": "summoner", "name": "召唤师"}]}
        if command == "game.panel.open":
            assert args["id"] == "summoner"
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
        if command == "packets.page":
            total = 30 if self.session == "fixture-one" else 1
            before = args.get("after") or total + 1
            numbers = [n for n in range(total, 0, -1)
                       if n < before and (not args.get("exportableOnly") or n % 2 == 0)]
            visible = numbers[:args["limit"]]
            return {
                "rows": [{"seq": n, "timeMs": 1800000000000 + n, "direction": "send" if n % 2 == 0 else "recv",
                          "socket": "9007199254740993", "cmd": f"{self.session}-{n:02}",
                          "summary": self.latest_summary if n == 30 else f"观察记录 <em>{n}</em>",
                          "length": 32, "decoded": True, "exportable": n % 2 == 0}
                         for n in visible],
                "oldestSeq": 1, "newestSeq": total, "nextAfter": visible[-1] if len(visible) < len(numbers) else None,
                "gap": False, "droppedRows": 0, "droppedChunks": 0, "gaps": 0, "retaining": self.retaining,
                "logFile": "logs/fixture.jsonl" if self.retaining else None, "logError": None, "droppedLogRows": 0,
            }
        if command == "packets.retain.set":
            self.retaining = args["enabled"]
            return {"requested": self.retaining}
        if command == "packets.export":
            assert args["seq"] % 2 == 0
            return {"seq": args["seq"], "text": f"offline export {args['seq']}"}
        if command == "battle.snapshot":
            details = self.battle or {"state": "unknown", "battleUniqueId": None,
                                      "selfPmmId": None, "lastCommand": None, "enemy": None}
            return {"state": details["state"], "battleId": 1, "switches": 0, "source": "offline",
                    "updatedMs": 1800000000000, "details": details}
        raise AssertionError(f"Unexpected panel command: {command}")


BRIDGE = """
window.myproGeneration = 3;
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
    fixture.retaining = False
    page.evaluate("""([seq,id]) => window.myproReceive({protocolVersion:1, kind:'event', bridgeGeneration:3,
       seq, event:'session.changed', sessionId:id, payload:{sessionId:id}})""", [fixture.session_events, session])
    expect(page.locator("#session")).to_have_text(session)


def packets(page, fixture):
    page.locator("[data-tab=packets]").click()
    rows = page.locator("#packet-rows")
    expect(rows).to_contain_text("fixture-one-30")
    expect(rows).not_to_contain_text("fixture-one-05")
    expect(rows.locator("em")).to_have_count(0)
    fixture.latest_summary = "最新记录已抵达"
    expect(rows).to_contain_text(fixture.latest_summary)
    page.locator("#packet-older").click()
    expect(rows).to_contain_text("fixture-one-01")
    expect(rows).not_to_contain_text("fixture-one-06")
    expect(page.locator("#packet-older")).to_be_disabled()
    page.locator("#packet-newer").click()
    expect(rows).to_contain_text("fixture-one-30")
    page.locator("#packet-exportable").check()
    expect(rows).to_contain_text("fixture-one-02")
    expect(rows).not_to_contain_text("fixture-one-29")
    page.locator("#packet-retain").click()
    expect(page.locator("#packets")).to_contain_text("fixture.jsonl")
    assert fixture.retaining
    page.locator("#packet-retain").click()
    expect(page.locator("#packets")).not_to_contain_text("fixture.jsonl")
    assert not fixture.retaining
    rows.get_by_role("button").first.click()
    exported = page.locator("#packet-text")
    assert exported.evaluate("(el) => el.readOnly")
    expect(exported).to_have_value("offline export 30")

    # An old reply can reach JavaScript after the host announces a new Player.
    page.evaluate("window.holdNext = 'packets.export'")
    rows.get_by_role("button").first.click()
    page.wait_for_function("typeof window.heldReply === 'function'")
    switch_session(page, fixture, "fixture-two")
    page.evaluate("window.heldReply()")
    expect(exported).to_have_value("")
    page.locator("#packet-exportable").uncheck()
    expect(rows).to_contain_text("fixture-two-01")
    expect(rows).not_to_contain_text("fixture-one")
    assert any(call[0] == "packets.page" and call[2] == fixture.session
               and "after" not in call[1] for call in fixture.calls)


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
    assert fixture.display == {"hideBattleDamage": True, "hideBattleHealing": True, "skipBattleAnimations": True}


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
        packets(page, fixture)
        battle(page, fixture)
        presentation(page, fixture)
        page.locator("#theme-choice").select_option("10")
        page.locator("#theme-apply").click()
        expect(page.locator("#theme-status")).to_contain_text("待战斗结束")
        page.locator("[data-tab=workshop]").click()
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
    print("PASS packet logs, enemy slots, session isolation, battle presentation, themes, workshop and lineup controls")


if __name__ == "__main__":
    main()
