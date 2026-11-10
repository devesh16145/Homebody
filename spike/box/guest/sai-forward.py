#!/usr/bin/env python3
"""sai-forward: the guest's only way out.

Listens on 127.0.0.1:3128 inside the Box and relays each TCP connection to the host over
vsock (CID 2, port 3128), where the Sandbox-ai proxy decides by hostname. Programs just use
http_proxy / https_proxy = http://127.0.0.1:3128. No NIC exists, so nothing can bypass this.
"""
import socket, threading, sys

LISTEN = ("127.0.0.1", 3128)
HOST_CID, HOST_PORT = 2, 3128

def pump(src, dst):
    try:
        while True:
            data = src.recv(65536)
            if not data:
                break
            dst.sendall(data)
    except OSError:
        pass
    finally:
        try: dst.shutdown(socket.SHUT_WR)
        except OSError: pass

def handle(client):
    try:
        up = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
        up.connect((HOST_CID, HOST_PORT))
    except OSError as e:
        client.sendall(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
        client.close()
        print("vsock connect failed:", e, file=sys.stderr, flush=True)
        return
    t = threading.Thread(target=pump, args=(up, client), daemon=True)
    t.start()
    pump(client, up)
    t.join()
    client.close(); up.close()

def main():
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(LISTEN)
    srv.listen(64)
    print("sai-forward listening on %s:%d -> vsock %d:%d" % (LISTEN + (HOST_CID, HOST_PORT)), flush=True)
    while True:
        c, _ = srv.accept()
        threading.Thread(target=handle, args=(c,), daemon=True).start()

if __name__ == "__main__":
    main()
