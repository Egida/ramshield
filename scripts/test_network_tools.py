#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).parent

def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_canonical_networks():
    get = load("network_get")
    assert get.canonical_network("192.0.2.17", 24) == "192.0.2.0/24"
    assert get.canonical_network("2001:db8::1234", 64) == "2001:db8::/64"


def test_fixture_addresses_match_network_create_contract():
    create = load("network_create")
    plan = create.build_plan(2, "v4", 24, 4, 7)
    assert plan[0]["network"] == "192.0.2.0/24"
    assert all(1 <= int(item["ip"].split(".")[-1]) <= 254 for item in plan[0]["hosts"])
    assert len({item["network"] for item in plan}) == 2


def test_config_overlay_uses_isolated_test_binds():
    create = load("network_create")
    overlay = create.ramshield_overlay("xdp_test0", "17890", "19999")
    assert overlay["ipc"]["tcp_addr"] == "127.0.0.1:17890"
    assert overlay["dashboard"]["http_addr"] == "127.0.0.1:19999"
    assert overlay["xdp"]["interface"] == "xdp_test0"


if __name__ == "__main__":
    for name, fn in sorted(globals().items()):
        if name.startswith("test_"):
            fn()
    print("network_tools: PASS")
