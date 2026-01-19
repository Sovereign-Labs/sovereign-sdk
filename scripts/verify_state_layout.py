#!/usr/bin/env python3
import json
import sys


def fail(message: str) -> None:
    print(f"state layout check failed: {message}", file=sys.stderr)
    sys.exit(1)


def load_layout(path: str) -> dict:
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except FileNotFoundError:
        fail(f"missing file: {path}")
    except json.JSONDecodeError as exc:
        fail(f"invalid json in {path}: {exc}")


def describe_module(module: dict) -> str:
    name = module.get("name", "<unknown>")
    discriminant = module.get("discriminant", "<unknown>")
    return f"{name} (discriminant {discriminant})"


def compare_state_items(old_module: dict, new_module: dict) -> None:
    old_items = old_module.get("state_items", [])
    new_items = new_module.get("state_items", [])

    if len(old_items) > len(new_items):
        fail(
            f"module {describe_module(old_module)} lost state items "
            f"(old {len(old_items)} > new {len(new_items)})"
        )

    for idx, old_item in enumerate(old_items):
        new_item = new_items[idx]
        if old_item.get("name") != new_item.get("name") or old_item.get(
            "discriminant"
        ) != new_item.get("discriminant"):
            fail(
                "module {module} state item changed at index {idx}: "
                "old {old_name}={old_disc}, new {new_name}={new_disc}".format(
                    module=describe_module(old_module),
                    idx=idx,
                    old_name=old_item.get("name"),
                    old_disc=old_item.get("discriminant"),
                    new_name=new_item.get("name"),
                    new_disc=new_item.get("discriminant"),
                )
            )


def compare_modules(old_layout: dict, new_layout: dict) -> None:
    old_modules = old_layout.get("modules", [])
    new_modules = new_layout.get("modules", [])

    if len(old_modules) > len(new_modules):
        fail(
            f"module list shrunk (old {len(old_modules)} > new {len(new_modules)})"
        )

    for idx, old_module in enumerate(old_modules):
        new_module = new_modules[idx]
        if old_module.get("name") != new_module.get("name") or old_module.get(
            "discriminant"
        ) != new_module.get("discriminant"):
            fail(
                "module changed at index {idx}: old {old}, new {new}".format(
                    idx=idx,
                    old=describe_module(old_module),
                    new=describe_module(new_module),
                )
            )
        compare_state_items(old_module, new_module)


def main() -> None:
    if len(sys.argv) != 3:
        fail("usage: verify_state_layout.py <old_layout.json> <new_layout.json>")

    old_layout = load_layout(sys.argv[1])
    new_layout = load_layout(sys.argv[2])

    compare_modules(old_layout, new_layout)
    print("state layout check passed")


if __name__ == "__main__":
    main()
