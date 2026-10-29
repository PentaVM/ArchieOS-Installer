"""The Windows half of omni-bridge: keeps a few connections open to the bridge in WSL (through
WSL's localhost forwarding) and, when one carries a request, opens a connection to the server
on Windows' own loopback and relays both ways. Nothing listens on Windows; no firewall rule.

    pythonw.exe omni-agent.py <bridge agent port in WSL> <server port on Windows> [slots]

Started and stopped with serve-nemotron-omni.ps1.
"""
import socket
import sys
import threading
import time

BRIDGE_PORT, SERVER_PORT = int(sys.argv[1]), int(sys.argv[2])
SLOTS = int(sys.argv[3]) if len(sys.argv) > 3 else 4


def relay(src, dst):
    try:
        while data := src.recv(65536):
            dst.sendall(data)
    except OSError:
        pass
    finally:
        for s in (src, dst):
            try:
                s.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass


def slot():
    backoff = 0.5
    while True:
        try:
            bridge = socket.create_connection(("127.0.0.1", BRIDGE_PORT), timeout=5)
        except OSError:
            time.sleep(backoff)
            backoff = min(backoff * 2, 10)
            continue
        backoff = 0.5
        bridge.settimeout(None)
        try:
            first = bridge.recv(65536)  # idle until the bridge pairs a client with us
        except OSError:
            first = b""
        if not first:
            bridge.close()
            continue
        try:
            server = socket.create_connection(("127.0.0.1", SERVER_PORT), timeout=10)
            server.settimeout(None)
            server.sendall(first)
        except OSError:
            bridge.close()
            continue
        back = threading.Thread(target=relay, args=(server, bridge), daemon=True)
        back.start()
        relay(bridge, server)
        back.join()
        bridge.close()
        server.close()


for _ in range(SLOTS):
    threading.Thread(target=slot, daemon=True).start()
threading.Event().wait()
