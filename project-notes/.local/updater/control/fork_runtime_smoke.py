"""Offline title, selector and real loopback-handshake regression gate.

Uses the exact candidate binaries and fresh player directories. Never reads Prism.
This verifies protocol selection, not a complete multiplayer playthrough.
"""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import threading
import time
from lab import game_env


def varint(value):
    result = bytearray()
    while value > 127:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return result


def read_varint(connection):
    value = 0
    for bit in range(0, 35, 7):
        data = connection.recv(1)
        if not data:
            raise EOFError("Incomplete handshake")
        value |= (data[0] & 127) << bit
        if not data[0] & 128:
            return value
    raise ValueError("Invalid VarInt")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binaries", type=Path, required=True)
    parser.add_argument("--resources", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    # Keep this regression focused on the custom selector, using a fresh local
    # options file instead of inheriting any real player's onboarding state.
    (output / "options.txt").write_text(
        "onboardAccessibility:false\nskipMultiplayerWarning:true\nlang:en_us\n")
    env = {k: v for k, v in os.environ.items() if not k.startswith("HORIZON_")}
    for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME",
                "HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"):
        directory = output / key.lower()
        directory.mkdir()
        env[key] = str(directory)
    env = game_env(env, HORIZON_OLDER_SERVERS="1", HORIZON_PLAYER_NAME="UpdaterTest")
    suffix = ".exe" if os.name == "nt" else ""
    game = args.binaries.resolve() / ("horizon-client" + suffix)
    control_tool = args.binaries.resolve() / ("horizon-ctl" + suffix)
    control = output / "control.json"
    flags = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}

    def ctl(*command):
        response = subprocess.run([str(control_tool), "--socket", str(control), "--timeout", "10", *command],
                                  cwd=output, env=env, capture_output=True, text=True, timeout=20, **flags)
        if response.returncode:
            raise RuntimeError("Control command failed: " + " ".join(command) + "\n" + response.stdout + response.stderr)
        return json.loads(response.stdout)

    def wait_for(predicate, timeout=90):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if client.poll() is not None:
                raise RuntimeError("Candidate exited before runtime acceptance")
            try:
                value = predicate()
                if value:
                    return value
            except (RuntimeError, KeyError, json.JSONDecodeError):
                pass
            time.sleep(0.25)
        raise TimeoutError("Runtime acceptance timed out")

    results = []
    with (output / "stdout.log").open("w") as stdout, (output / "stderr.log").open("w") as stderr:
        client = subprocess.Popen([str(game), "--resources", str(args.resources.resolve()),
                                   "--game-dir", str(output), "--home", env["HORIZON_HOME"],
                                   "--name", "UpdaterTest", "--offscreen", "--control", "--control-socket",
                                   str(control), "--width", "1280", "--height", "720"],
                                  cwd=output, env=env, stdout=stdout, stderr=stderr, **flags)
        try:
            wait_for(lambda: control.exists() and ctl("state").get("screen") == "title")
            wait_for(lambda: any(w.get("label") == "Multiplayer" and w.get("enabled")
                                 for w in ctl("ui")["widgets"]))
            ctl("click-button", "Multiplayer")
            for label, query, expected in (("26.3", "26.3", 777), ("1.21.4", "Garden", 769)):
                widgets = ctl("ui")["widgets"]
                button = next(w["label"] for w in widgets if w.get("label") in ("ViaHorizon", "ViaFabricPlus"))
                ctl("click-button", button)
                wait_for(lambda: any(w.get("label") == "Modern" for w in ctl("ui")["widgets"]))
                ctl("click-at", "213", "46")
                ctl("type", query)
                row = wait_for(lambda: next((w for w in ctl("ui")["widgets"]
                                             if w.get("label") == label and w.get("enabled")), None))
                x, y, width, height = row["bounds"]
                ctl("click-at", str(x + width / 2), str(y + height / 2))
                ctl("click-button", "Back")
                observed = {}
                with socket.socket() as listener:
                    listener.bind(("127.0.0.1", 0))
                    listener.listen()
                    listener.settimeout(15)

                    def observe():
                        try:
                            with listener.accept()[0] as connection:
                                connection.settimeout(10)
                                read_varint(connection)
                                observed["packet"] = read_varint(connection)
                                observed["protocol"] = read_varint(connection)
                                reason = b'{"text":"Updater loopback acceptance"}'
                                body = varint(0) + varint(len(reason)) + reason
                                connection.sendall(varint(len(body)) + body)
                                time.sleep(0.3)
                        except Exception as error:
                            observed["error"] = type(error).__name__

                    worker = threading.Thread(target=observe, daemon=True)
                    worker.start()
                    ctl("connect", f"127.0.0.1:{listener.getsockname()[1]}")
                    worker.join(timeout=20)
                if worker.is_alive() or observed != {"packet": 0, "protocol": expected}:
                    raise AssertionError(f"Incorrect {label} handshake: {observed}")
                results.append({"version": label, "protocol": expected})
                wait_for(lambda: any(b.get("label") == "Back to Server List"
                                    for b in ctl("screen").get("buttons", [])), timeout=20)
                ctl("click-button", "Back to Server List")
            ctl("screenshot", str(output / "multiplayer.png"))
            ctl("quit")
            if client.wait(timeout=20) != 0:
                raise RuntimeError("Candidate did not exit cleanly")
            report = {"passed": True, "handshakes": results,
                      "scope": "offline UI and loopback handshake; no authenticated gameplay"}
            (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps(report))
        finally:
            if client.poll() is None:
                client.kill()
                client.wait(timeout=10)


if __name__ == "__main__":
    main()
