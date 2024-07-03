import os
import asyncio
import random
import subprocess

from typing import Optional


__all__ = ["LSPServerRunner"]


def localhost_port_not_in_use(start: int, stop: int):
    def _is_port_in_use(port: int) -> bool:
        import socket
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            return s.connect_ex(('localhost', port)) == 0

    ports_range = list(range(start, stop))
    random.shuffle(ports_range)
    for port in ports_range:
        if not _is_port_in_use(port):
            return port

    raise RuntimeError(f"cannot find port in range [{start}, {stop})")


class LSPServerRunner:
    def __init__(self, repo_path: str):
        base_command = os.environ["REFACT_LSP_BASE_COMMAND"]
        port = localhost_port_not_in_use(8100, 9000)
        self._command = [
            *base_command.split(" "),
            "--logs-stderr", f"--http-port={port}",
            f"--workspace-folder={repo_path}",
            "--ast",
        ]

        self._port: int = port
        self._lsp_server: Optional[asyncio.subprocess.Process] = None

    @property
    def _is_lsp_server_running(self) -> bool:
        return self._lsp_server is not None and self._lsp_server.returncode is None

    @property
    def base_url(self):
        return f"http://127.0.0.1:{self._port}/v1"

    async def _start(self):
        self._lsp_server = await asyncio.create_subprocess_exec(
            *self._command, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

        while True:
            stderr = await self._lsp_server.stderr.readline()
            if "AST COMPLETE" in stderr.decode():
                break
            if not self._is_lsp_server_running:
                raise RuntimeError(f"LSP server unexpectedly exited, bb")
            await asyncio.sleep(0.01)
        assert self._is_lsp_server_running

    async def _stop(self):
        if self._lsp_server is not None:
            self._lsp_server.terminate()
            await self._lsp_server.wait()
        assert not self._is_lsp_server_running

    async def __aenter__(self):
        await self._start()
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        await self._stop()
