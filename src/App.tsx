import { useEffect, useState } from "react";
import { commands } from "./bindings";
import type { UsbDevice } from "./bindings";
import "./App.css";

function speedLabel(speed: UsbDevice["speed"]): string {
  if (speed === "Low") return "Low Speed";
  if (speed === "Full") return "Full Speed";
  if (speed === "High") return "High Speed";
  if (speed === "Super") return "SuperSpeed";
  if (speed === "SuperPlus") return "SuperSpeed+";
  return "Unknown";
}

function productName(d: UsbDevice): string {
  const dd = d.device_descriptor;
  const s = d.strings.find((s) => s.index === dd.i_product);
  return s?.value ?? "(no product string)";
}

function manufacturerName(d: UsbDevice): string {
  const dd = d.device_descriptor;
  const s = d.strings.find((s) => s.index === dd.i_manufacturer);
  return s?.value ?? "";
}

function vidPid(d: UsbDevice): string {
  const dd = d.device_descriptor;
  return `${dd.id_vendor.toString(16).padStart(4, "0").toUpperCase()}:${dd.id_product.toString(16).padStart(4, "0").toUpperCase()}`;
}

function DeviceRow({ device }: { device: UsbDevice }) {
  const [open, setOpen] = useState(false);
  const mfr = manufacturerName(device);

  return (
    <div className="device">
      <div className="device-header" onClick={() => setOpen((o) => !o)}>
        <span className="speed">{speedLabel(device.speed)}</span>
        <span className="vidpid">{vidPid(device)}</span>
        <span className="product">{productName(device)}</span>
        {mfr && <span className="mfr">({mfr})</span>}
        <span className="toggle">{open ? "▾" : "▸"}</span>
      </div>
      {open && (
        <pre className="device-json">
          {JSON.stringify(device, null, 2)}
        </pre>
      )}
    </div>
  );
}

export default function App() {
  const [devices, setDevices] = useState<UsbDevice[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      const result = await commands.enumerateUsb();
      if (result.status === "ok") {
        setDevices(result.data);
      } else {
        setError(String(result.error));
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { refresh(); }, []);

  return (
    <div className="app">
      <header>
        <h1>USBProbester</h1>
        <button onClick={refresh} disabled={loading}>
          {loading ? "Refreshing…" : "Refresh"}
        </button>
      </header>

      {error && <div className="error">{error}</div>}

      <div className="device-list">
        {devices.length === 0 && !loading && !error && (
          <div className="empty">No USB devices found.</div>
        )}
        {devices.map((d) => (
          <DeviceRow key={d.location_id} device={d} />
        ))}
      </div>
    </div>
  );
}
