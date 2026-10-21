"""Camera bridge for archie-iris-visor: where the camera is, frames to the visor.

Under WSL the Linux VM sees no USB camera, so this runs on Windows, next to the camera, and
sends each frame as a JPEG to the visor listening in WSL (WSL forwards Windows' localhost to
the VM's loopback). Nothing leaves the machine and nothing is written to disk.

    python camera_bridge.py [--index N] [--width 1280]
                            [--height 720] [--fps 60] [--quality 80] [--to 127.0.0.1:7671]

Wire format: a 4-byte big-endian length, then the JPEG. Reconnects when the visor restarts.
"""
import argparse
import socket
import struct
import sys
import time

import cv2


def open_camera(args):
    """The camera at --index, or the first DirectShow camera that opens."""
    indices = [args.index] if args.index is not None else range(0, 6)
    for index in indices:
        capture = cv2.VideoCapture(index, cv2.CAP_DSHOW)
        if not capture.isOpened():
            capture.release()
            continue
        capture.set(cv2.CAP_PROP_FOURCC, cv2.VideoWriter_fourcc(*"MJPG"))
        capture.set(cv2.CAP_PROP_FRAME_WIDTH, args.width)
        capture.set(cv2.CAP_PROP_FRAME_HEIGHT, args.height)
        capture.set(cv2.CAP_PROP_FPS, args.fps)
        ok, frame = capture.read()
        if ok and frame is not None:
            height, width = frame.shape[:2]
            print(f"camera {index}: {width}x{height} at up to {capture.get(cv2.CAP_PROP_FPS):.0f} fps",
                  flush=True)
            return capture
        capture.release()
    sys.exit("no camera opened; pass --index")


def connect(host, port):
    while True:
        try:
            sock = socket.create_connection((host, port), timeout=5)
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            sock.settimeout(None)
            print(f"sending to {host}:{port}", flush=True)
            return sock
        except OSError:
            time.sleep(1)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--index", type=int)
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=720)
    parser.add_argument("--fps", type=int, default=60)
    parser.add_argument("--quality", type=int, default=80)
    parser.add_argument("--to", default="127.0.0.1:7671")
    args = parser.parse_args()
    host, port = args.to.rsplit(":", 1)
    capture = open_camera(args)
    encode = [int(cv2.IMWRITE_JPEG_QUALITY), max(30, min(95, args.quality))]
    sock = connect(host, int(port))
    sent, since = 0, time.monotonic()
    while True:
        ok, frame = capture.read()
        if not ok:
            time.sleep(0.01)
            continue
        ok, jpeg = cv2.imencode(".jpg", frame, encode)
        if not ok:
            continue
        data = jpeg.tobytes()
        try:
            sock.sendall(struct.pack(">I", len(data)) + data)
        except OSError:
            sock.close()
            sock = connect(host, int(port))
            continue
        sent += 1
        if time.monotonic() - since >= 5:
            print(f"{sent / (time.monotonic() - since):.1f} fps sent", flush=True)
            sent, since = 0, time.monotonic()


if __name__ == "__main__":
    main()
