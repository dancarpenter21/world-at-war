import { authorizedUrl, leaseGeneration } from "./apiClient";
import { useEffect, useState } from "react";
import { decodeNetworkFrame, type NetworkProjection } from "./networkModel";

export function useNetworkStream(apiBase: string, gameId: string, playerId: string, roleId: string) {
  const generation = leaseGeneration(gameId, roleId);
  const [projection, setProjection] = useState<NetworkProjection | null>(null);
  const [status, setStatus] = useState<"connecting" | "live" | "reconnecting">("connecting");
  const [notice, setNotice] = useState("");
  useEffect(() => {
    let active = true;
    let socket: WebSocket | undefined;
    let reconnectTimer: number | undefined;
    let lastSequence: number | undefined;
    let attempts = 0;
    setProjection(null);
    setStatus("connecting");
    setNotice("");
    const reconnect = () => {
      if (!active) return;
      setStatus("reconnecting");
      setNotice((current) => current || "Connection interrupted. Showing the last received topology while reconnecting.");
      reconnectTimer = window.setTimeout(connect, Math.min(1_000 * 2 ** attempts++, 10_000));
    };
    const connect = () => {
      if (!active) return;
      const url = apiBase ? new URL(apiBase, window.location.href) : new URL(window.location.href);
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      url.pathname = `/v1/games/${gameId}/network/stream`;
      url.search = "";
      url.hash = "";
      url.searchParams.set("player_id", playerId);
      url.searchParams.set("role_id", roleId);
      if (lastSequence !== undefined) url.searchParams.set("after_sequence", String(lastSequence));
      socket = new WebSocket(authorizedUrl(url.toString()));
      socket.onmessage = (event) => {
        if (!active) return;
        const frame = decodeNetworkFrame(String(event.data));
        if (!frame) {
          setNotice("An unreadable network update was received. Restoring the current topology.");
          lastSequence = undefined;
          socket?.close(4000, "Invalid network frame");
          return;
        }
        if (!frame.resync && lastSequence !== undefined && frame.sequence <= lastSequence) return;
        lastSequence = frame.sequence;
        attempts = 0;
        setProjection(frame.projection);
        setStatus("live");
        setNotice("");
      };
      socket.onerror = () => {
        if (active) setNotice("Network stream unavailable. Showing the last received topology while reconnecting.");
      };
      socket.onclose = (event) => {
        if (event.code === 1008) {
          setProjection(null); setNotice("Session or role lease is no longer valid.");
          window.dispatchEvent(new Event("role-lease-lost"));
        } else reconnect();
      };
    };
    connect();
    return () => {
      active = false;
      if (reconnectTimer !== undefined) window.clearTimeout(reconnectTimer);
      if (socket) { socket.onclose = null; socket.close(); }
    };
  }, [apiBase, gameId, playerId, roleId, generation]);
  return { projection, status, notice };
}
