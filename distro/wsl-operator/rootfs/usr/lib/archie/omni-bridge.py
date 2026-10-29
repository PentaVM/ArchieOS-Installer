#!/usr/bin/env python3
"""Reach a server on Windows' loopback from inside WSL, with neither a port opened in Windows'
firewall nor WSL interop (which ArchieOS turns off): the connections are made from Windows.

omni-agent.py, on Windows, keeps a few connections open to this bridge's agent port (Windows
reaches WSL's loopback through WSL's own localhost forwarding). When a client connects to the
client port, it is paired with one of them, and the agent carries it to the server on Windows'
127.0.0.1.

    omni-bridge.py <client port> <agent port>

Used by archie-omni for Nemotron Omni, served from Windows where the memory is.
"""
import asyncio
import sys

CLIENT_PORT, AGENT_PORT = int(sys.argv[1]), int(sys.argv[2])
WAIT_FOR_AGENT_S = 10
idle: asyncio.Queue = asyncio.Queue()


async def agent_arrived(reader, writer):
    await idle.put((reader, writer))


async def pipe(reader, writer):
    try:
        while data := await reader.read(65536):
            writer.write(data)
            await writer.drain()
    except (ConnectionError, OSError):
        pass
    finally:
        try:
            writer.close()
        except OSError:
            pass


async def client_arrived(reader, writer):
    while True:
        try:
            agent_reader, agent_writer = await asyncio.wait_for(idle.get(), WAIT_FOR_AGENT_S)
        except asyncio.TimeoutError:
            writer.close()  # no agent: Nemotron Omni is not running on Windows
            return
        if not agent_writer.is_closing() and not agent_reader.at_eof():
            break
    await asyncio.gather(pipe(reader, agent_writer), pipe(agent_reader, writer))


async def main():
    clients = await asyncio.start_server(client_arrived, "127.0.0.1", CLIENT_PORT)
    agents = await asyncio.start_server(agent_arrived, "127.0.0.1", AGENT_PORT)
    async with clients, agents:
        await asyncio.gather(clients.serve_forever(), agents.serve_forever())


asyncio.run(main())
