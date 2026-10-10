import { vi } from "vitest";
import { TerminalConnection } from "./terminal";

/** A WebSocket the test drives by hand. */
class FakeSocket extends EventTarget {
  sent: (string | Uint8Array)[] = [];
  closed: number | null = null;
  send(data: string | Uint8Array) {
    this.sent.push(data);
  }
  close(code: number) {
    this.closed = code;
  }
  opens() {
    this.dispatchEvent(new Event("open"));
  }
  says(data: string | ArrayBuffer) {
    this.dispatchEvent(new MessageEvent("message", { data }));
  }
  drops() {
    this.dispatchEvent(new Event("close"));
  }
}

function connect(size = { cols: 80, rows: 24 }) {
  const socket = new FakeSocket();
  const printed: string[] = [];
  const open = vi.fn(() => socket as unknown as WebSocket);
  const connection = new TerminalConnection(
    open,
    (data) => printed.push(new TextDecoder().decode(data)),
    size,
  );
  connection.start();
  return { socket, printed, open, connection };
}

const bytes = (text: string) => new TextEncoder().encode(text).buffer;

describe("TerminalConnection", () => {
  it("opens the socket with the screen it was given and prints what arrives", () => {
    const { socket, printed, open, connection } = connect({ cols: 120, rows: 40 });
    expect(open).toHaveBeenCalledWith({ cols: 120, rows: 40 });
    expect(connection.getSnapshot().state).toBe("connecting");
    socket.opens();
    expect(connection.getSnapshot().state).toBe("open");
    socket.says(bytes("$ "));
    socket.says(bytes("ls\r\n"));
    expect(printed).toEqual(["$ ", "ls\r\n"]);
  });

  it("sends what is typed as bytes, and only once open", () => {
    const { socket, connection } = connect();
    connection.type("early");
    socket.opens();
    connection.type("ls\n");
    connection.type(new Uint8Array([0x1b, 0x5b, 0x41]));
    expect(socket.sent).toEqual([
      new TextEncoder().encode("ls\n"),
      new Uint8Array([0x1b, 0x5b, 0x41]),
    ]);
  });

  it("sends a resize when the screen changes while open, and not before", () => {
    const { socket, connection } = connect();
    connection.resize({ cols: 100, rows: 30 });
    expect(socket.sent).toEqual([]);
    socket.opens();
    connection.resize({ cols: 132, rows: 43 });
    connection.resize({ cols: 132, rows: 43 });
    expect(socket.sent).toEqual([JSON.stringify({ type: "resize", cols: 132, rows: 43 })]);
  });

  it("ends with the exit code of the process", () => {
    const { socket, connection } = connect();
    socket.opens();
    socket.says(JSON.stringify({ type: "exit", code: 7 }));
    socket.drops();
    expect(connection.getSnapshot()).toMatchObject({ state: "exited", end: { code: 7 } });
  });

  it("ends with the reason when the process left no exit code", () => {
    const { socket, connection } = connect();
    socket.opens();
    socket.says(JSON.stringify({ type: "exit", failure: "lost" }));
    expect(connection.getSnapshot()).toMatchObject({ state: "exited", end: { failure: "lost" } });
  });

  it("fails when the socket closes before it opened and closes when it drops open", () => {
    const refused = connect();
    refused.socket.drops();
    expect(refused.connection.getSnapshot().state).toBe("failed");

    const dropped = connect();
    dropped.socket.opens();
    dropped.socket.drops();
    expect(dropped.connection.getSnapshot().state).toBe("closed");
  });

  it("fails when the socket cannot be created", () => {
    const connection = new TerminalConnection(
      () => {
        throw new SyntaxError("bad subprotocol");
      },
      () => {},
      { cols: 80, rows: 24 },
    );
    connection.start();
    expect(connection.getSnapshot().state).toBe("failed");
  });

  it("closes its socket normally, which ends the process", () => {
    const { socket, connection } = connect();
    socket.opens();
    connection.close();
    expect(socket.closed).toBe(1000);
    expect(connection.getSnapshot().state).toBe("closed");
    connection.type("ignored");
    expect(socket.sent).toEqual([]);
  });

  it("notifies subscribers of each change", () => {
    const { socket, connection } = connect();
    const seen = vi.fn();
    connection.subscribe(seen);
    socket.opens();
    socket.says(JSON.stringify({ type: "exit", code: 0 }));
    expect(seen).toHaveBeenCalledTimes(2);
  });
});
