import React, { useEffect, useRef, useState, createContext, useContext } from "react";
import type { ReactNode } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";

const DepthCtx = createContext(0);
import { commands } from "./bindings";
import type {
  UsbDevice_Serialize,
  ConfigDescriptor_Serialize,
  InterfaceDescriptor,
  EndpointDescriptor,
  HidInterface_Serialize,
  HidNode,
  HidIoFlags,
  CollectionKind,
  ClassSpecificDescriptor,
  IadDescriptor,
} from "./bindings";
import "./App.css";

// ── helpers ───────────────────────────────────────────────────────────────────

// mirrors speed_label() in crates/usb-formatter/src/lib.rs
function speedLabel(speed: UsbDevice_Serialize["speed"]): string {
  if (speed === "Low") return "Low Speed";
  if (speed === "Full") return "Full Speed";
  if (speed === "High") return "High Speed";
  if (speed === "Super") return "SuperSpeed";
  if (speed === "SuperPlus") return "SuperSpeed+";
  return "Unknown";
}

function getString(device: UsbDevice_Serialize, index: number): string | undefined {
  return device.strings.find((s) => s.index === index)?.value;
}

function lastPort(device: UsbDevice_Serialize): number {
  const p = device.port_path;
  return p[p.length - 1] ?? 0;
}

function hex2(n: number): string {
  return n.toString(16).padStart(2, "0").toUpperCase();
}

function hex4(n: number): string {
  return n.toString(16).padStart(4, "0").toUpperCase();
}

function hexDump(bytes: number[]): string {
  const rows: string[] = [];
  for (let i = 0; i < bytes.length; i += 16) {
    const chunk = bytes.slice(i, i + 16);
    const hex = chunk.map((b) => hex2(b)).join(" ");
    rows.push(`${i.toString(16).padStart(4, "0")}: ${hex}`);
  }
  return rows.join("\n");
}

// mirrors class_label() in crates/usb-formatter/src/lib.rs
function deviceClassLabel(cls: number): string {
  const labels: Record<number, string> = {
    0: "Composite", 1: "Audio", 2: "Communications", 3: "HID",
    5: "Physical", 6: "Still Imaging", 7: "Printer", 8: "Mass Storage",
    9: "Hub", 10: "CDC Data", 11: "Smart Card", 14: "Video",
    224: "Wireless", 239: "Misc", 254: "Application Specific", 255: "Vendor Specific",
  };
  return labels[cls] ?? `0x${hex2(cls)}`;
}

function interfaceClassLabel(cls: number, sub: number): string {
  if (cls === 1) {
    if (sub === 1) return "Audio/Control";
    if (sub === 2) return "Audio/Streaming";
    if (sub === 3) return "Audio/MIDI Streaming";
    return "Audio";
  }
  if (cls === 2) return "Communications-Control";
  if (cls === 10) return "Communications-Data";
  if (cls === 8) return "Mass Storage/SCSI";
  return deviceClassLabel(cls);
}

// mirrors subclass_label() in crates/usb-formatter/src/lib.rs
function interfaceSubclassLabel(cls: number, sub: number): string {
  const map: Record<number, Record<number, string>> = {
    1:  { 1: "Control", 2: "Streaming", 3: "MIDI Streaming" },
    2:  { 1: "Direct Line", 2: "Abstract (Modem)", 3: "Telephone",
          4: "Multi-Channel", 5: "CAPI", 6: "Ethernet Networking", 7: "ATM Networking" },
    3:  { 0: "No Subclass", 1: "Boot Interface" },
    8:  { 1: "Reduced Block Commands", 2: "SFF-8020i", 3: "QIC-157",
          4: "UFI", 5: "SFF-8070i", 6: "SCSI Transparent" },
    14: { 1: "Video Control", 2: "Video Streaming", 3: "Video Interface Collection" },
  };
  return map[cls]?.[sub] ?? "";
}

// mirrors xfer_label() in crates/usb-formatter/src/lib.rs
function endpointType(attr: number): string {
  const t = ["Control", "Isochronous", "Bulk", "Interrupt"];
  return t[attr & 0x03] ?? "Unknown";
}

function endpointDir(addr: number): string {
  return addr & 0x80 ? "Input" : "Output";
}

function collectionKindLabel(kind: CollectionKind): string {
  if (typeof kind === "string") {
    return kind.replace(/([A-Z])/g, " $1").trim();
  }
  if ("Vendor" in kind) return `Vendor (0x${kind.Vendor.id.toString(16)})`;
  return "Unknown";
}

function hidFlagsLabel(flags: HidIoFlags, kind: "input" | "output" | "feature"): string {
  const isInput = kind === "input";
  const parts: string[] = [];
  parts.push(flags.constant ? "Constant" : "Data");
  parts.push(flags.variable ? "Variable" : "Array");
  parts.push(flags.relative ? "Relative" : "Absolute");

  if (isInput && !flags.variable) {
    // Array Input: HID spec says flags 3–8 are not applicable
    return `(${parts.join(", ")})`;
  }

  parts.push(flags.wrap ? "Wrap" : "No Wrap");
  parts.push(flags.non_linear ? "Non Linear" : "Linear");
  parts.push(flags.no_preferred ? "No Preferred State" : "Preferred State");
  parts.push(flags.null_state ? "Null State" : "No Null Position");
  if (!isInput) {
    // Volatile is not applicable to Input
    parts.push(flags.volatile ? "Volatile" : "Nonvolatile");
  }
  parts.push(flags.buffered_bytes ? "Buffered Bytes" : "Bitfield");

  return `(${parts.join(", ")})`;
}

// mirrors config_attrs_desc() in crates/usb-formatter/src/lib.rs
function attrsLabel(attr: number): string {
  const bits: string[] = [];
  if (attr & 0x40) bits.push("self-powered");
  if (attr & 0x20) bits.push("remote wakeup");
  return `0x${hex2(attr)}${bits.length ? ` (${bits.join(", ")})` : ""}`;
}

// ── TreeNode ──────────────────────────────────────────────────────────────────

const LABEL_BASE = 450;
const INDENT = 16;

function TreeNode({
  label,
  value,
  defaultOpen = false,
  children,
}: {
  label: string;
  value?: string;
  defaultOpen?: boolean;
  children?: ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const depth = useContext(DepthCtx);
  const hasChildren = !!children;
  const labelWidth = Math.max(0, LABEL_BASE - depth * INDENT);

  return (
    <div className="tree-node">
      <div
        className={`tree-row${hasChildren ? " clickable" : ""}`}
        onClick={hasChildren ? () => setOpen((o) => !o) : undefined}
      >
        <span className="toggle">{hasChildren ? (open ? "▾" : "▸") : ""}</span>
        <span className="label-area" style={{ flex: `0 0 ${labelWidth}px` }}>
          <span className="lbl">{label}</span>
          {value !== undefined && <span className="dots" aria-hidden="true" />}
        </span>
        {value !== undefined && <span className="val">{value}</span>}
      </div>
      {open && (
        <DepthCtx.Provider value={depth + 1}>
          <div className="tree-children">{children}</div>
        </DepthCtx.Provider>
      )}
    </div>
  );
}

function Leaf({ label, value }: { label: string; value?: string }) {
  const depth = useContext(DepthCtx);
  const labelWidth = Math.max(0, LABEL_BASE - depth * INDENT);

  return (
    <div className="tree-node">
      <div className="tree-row">
        <span className="toggle" />
        <span className="label-area" style={{ flex: `0 0 ${labelWidth}px` }}>
          <span className="lbl">{label}</span>
          {value !== undefined && <span className="dots" aria-hidden="true" />}
        </span>
        {value !== undefined && <span className="val">{value}</span>}
      </div>
    </div>
  );
}

// ── HID tree ──────────────────────────────────────────────────────────────────

function HidNodeView({ node }: { node: HidNode }) {
  switch (node.type) {
    case "Collection":
      return (
        <TreeNode label={`Collection (${collectionKindLabel(node.kind)})`}>
          {node.children.map((child, i) => (
            <HidNodeView key={i} node={child} />
          ))}
        </TreeNode>
      );
    case "UsagePage": {
      const pid = node.page_id;
      const phex = `0x${pid.toString(16).padStart(4, "0").toUpperCase()}`;
      return <Leaf label="Usage Page" value={`(${node.name} ${phex} (${pid}))`} />;
    }
    case "Usage": {
      const uid = node.usage_id;
      const uhex = `0x${uid.toString(16).padStart(4, "0").toUpperCase()}`;
      const lbl = node.name ? `Usage (${node.name})` : "Usage";
      return <Leaf label={lbl} value={`${uhex} (${uid})`} />;
    }
    case "LogicalMinimum":  return <Leaf label="Logical Minimum" value={`${node.value}`} />;
    case "LogicalMaximum":  return <Leaf label="Logical Maximum" value={`${node.value}`} />;
    case "PhysicalMinimum": return <Leaf label="Physical Minimum" value={`${node.value}`} />;
    case "PhysicalMaximum": return <Leaf label="Physical Maximum" value={`${node.value}`} />;
    case "UnitExponent":    return <Leaf label="Unit Exponent" value={`${node.value}`} />;
    case "Unit":            return <Leaf label="Unit" value={`0x${node.value.toString(16)}`} />;
    case "ReportSize":      return <Leaf label="Report Size" value={`${node.value}`} />;
    case "ReportCount":     return <Leaf label="Report Count" value={`${node.value}`} />;
    case "ReportId":        return <Leaf label="Report ID" value={`${node.id}`} />;
    case "UsageMinimum":    return <Leaf label="Usage Minimum" value={`${node.value}`} />;
    case "UsageMaximum":    return <Leaf label="Usage Maximum" value={`${node.value}`} />;
    case "Input":           return <Leaf label="Input" value={hidFlagsLabel(node.flags, "input")} />;
    case "Output":          return <Leaf label="Output" value={hidFlagsLabel(node.flags, "output")} />;
    case "Feature":         return <Leaf label="Feature" value={hidFlagsLabel(node.flags, "feature")} />;
    default:                return null;
  }
}

// ── Descriptor nodes ──────────────────────────────────────────────────────────

function HexBlock({ bytes }: { bytes: number[] }) {
  return (
    <div className="tree-node">
      <div className="tree-row">
        <span className="toggle" />
        <pre className="hex-dump">{hexDump(bytes)}</pre>
      </div>
    </div>
  );
}

function EndpointNode({ ep }: { ep: EndpointDescriptor }) {
  const addr = ep.b_endpoint_address;
  const type = endpointType(ep.bm_attributes);
  const dir = endpointDir(addr);
  return (
    <TreeNode label={`Endpoint 0x${hex2(addr)} - ${type} ${dir}`}>
      <Leaf label="Address:" value={`0x${hex2(addr)}  (${dir})`} />
      <Leaf label="Attributes:" value={`0x${hex2(ep.bm_attributes)}  (${type})`} />
      <Leaf label="Max Packet Size:" value={`0x${hex4(ep.w_max_packet_size)}`} />
      <Leaf label="Polling Interval:" value={`${ep.b_interval}`} />
    </TreeNode>
  );
}

function HidDescriptorNode({ hid }: { hid: HidInterface_Serialize }) {
  const bytes = hid.raw_report_descriptor;
  return (
    <TreeNode label="HID Report Descriptor" defaultOpen>
      <TreeNode label={`Length (and contents):   ${bytes.length}`}>
        <HexBlock bytes={bytes} />
      </TreeNode>
      {hid.parsed && hid.parsed.length > 0 && (
        <TreeNode label="Parsed Report Descriptor:" defaultOpen>
          {hid.parsed.map((node, i) => (
            <HidNodeView key={i} node={node} />
          ))}
        </TreeNode>
      )}
    </TreeNode>
  );
}

function u16le(data: number[], off: number) {
  return data[off] | (data[off + 1] << 8);
}

function ClassSpecificNode({ cs, ifaceClass, ifaceSubClass }: { cs: ClassSpecificDescriptor; ifaceClass: number; ifaceSubClass: number }) {
  const { descriptor_type: typ, descriptor_subtype: sub, data } = cs;

  // HID class descriptor (0x21)
  if (typ === 0x21) {
    const bcdHID = u16le(data, 0);
    const hidDescTypeName: Record<number, string> = { 0x22: "Report", 0x23: "Physical" };
    return (
      <TreeNode label="HID Descriptor">
        <Leaf label="bcdHID:" value={hex4(bcdHID)} />
        <Leaf label="bCountryCode:" value={`${data[2]}`} />
        <Leaf label="bNumDescriptors:" value={`${data[3]}`} />
        {data[3] > 0 && <>
          <Leaf label="bDescriptorType:" value={`${hidDescTypeName[data[4]] ?? hex2(data[4])}`} />
          <Leaf label="wDescriptorLength:" value={`${u16le(data, 5)}`} />
        </>}
      </TreeNode>
    );
  }

  // CS_INTERFACE (0x24) — decode by interface class
  if (typ === 0x24) {
    // CDC (class 2)
    if (ifaceClass === 0x02) {
      const cdcSubtypeNames: Record<number, string> = {
        0x00: "Comm Class Header Functional Descriptor",
        0x01: "Comm Class Call Management Functional Descriptor",
        0x02: "Comm Class Abstract Control Management Functional Descriptor",
        0x03: "Comm Class Direct Line Management Functional Descriptor",
        0x06: "Comm Class Union Functional Descriptor",
        0x0F: "Comm Class MDLM Functional Descriptor",
        0x10: "Comm Class MDLM Detail Functional Descriptor",
      };
      const label = cdcSubtypeNames[sub] ?? `Comm Class-Specific Descriptor (subtype ${hex2(sub)})`;
      const rawHex = `0000: ${[typ === 0x24 ? data.length + 3 : data.length + 2, typ, sub, ...data].map(b => b.toString(16).padStart(2,"0").toUpperCase()).join(" ")}`;
      return (
        <TreeNode label={label}>
          <Leaf label="Raw Descriptor (hex)" value={rawHex} />
          {sub === 0x00 && data.length >= 2 && <Leaf label="bcdCDC:" value={hex4(u16le(data, 0))} />}
          {sub === 0x01 && data.length >= 2 && <>
            <Leaf label="bmCapabilities:" value={hex2(data[0])} />
            <Leaf label="bDataInterface:" value={`${data[1]}`} />
          </>}
          {sub === 0x02 && data.length >= 1 && <Leaf label="bmCapabilities:" value={hex2(data[0])} />}
          {sub === 0x06 && data.length >= 2 && <>
            <Leaf label="bMasterInterface:" value={`${data[0]}`} />
            {data.slice(1).map((v, i) => <Leaf key={i} label={`bSlaveInterface${i}:`} value={`${v}`} />)}
          </>}
        </TreeNode>
      );
    }

    // Audio class (class 1)
    if (ifaceClass === 0x01) {
      const jackType = (t: number) => t === 0x01 ? "Embedded" : t === 0x02 ? "External" : hex2(t);
      const rawHex = (d: number[]) => d.map(b => b.toString(16).padStart(2,"0").toUpperCase()).join(" ");

      // AudioControl (subclass 1)
      if (ifaceSubClass === 0x01 && sub === 0x01 && data.length >= 4) {
        const bcdADC = u16le(data, 0);
        const wTotalLength = u16le(data, 2);
        const bInCollection = data[4] ?? 0;
        const ifaces = data.slice(5, 5 + bInCollection);
        return (
          <TreeNode label="Audio Control Interface Header Descriptor">
            <Leaf label="bcdADC:" value={hex4(bcdADC)} />
            <Leaf label="wTotalLength:" value={`${wTotalLength}`} />
            <Leaf label="bInCollection:" value={`${bInCollection}`} />
            {Array.from(ifaces).map((v, i) => <Leaf key={i} label={`baInterfaceNr[${i}]:`} value={`${v}`} />)}
          </TreeNode>
        );
      }

      // MIDIStreaming (subclass 3)
      if (ifaceSubClass === 0x03) {
        if (sub === 0x01 && data.length >= 4) {
          const bcdMSC = u16le(data, 0);
          const wTotalLength = u16le(data, 2);
          return (
            <TreeNode label="MIDI Streaming Interface Header Descriptor">
              <Leaf label="bcdMSC:" value={hex4(bcdMSC)} />
              <Leaf label="wTotalLength:" value={`${wTotalLength}`} />
            </TreeNode>
          );
        }
        if (sub === 0x02 && data.length >= 3) {
          return (
            <TreeNode label="MIDI IN Jack Descriptor">
              <Leaf label="bJackType:" value={`${jackType(data[0])} (${hex2(data[0])})`} />
              <Leaf label="bJackID:" value={`${data[1]}`} />
              <Leaf label="iJack:" value={`${data[2]}`} />
            </TreeNode>
          );
        }
        if (sub === 0x03 && data.length >= 3) {
          const nPins = data[2];
          const srcPairs = Array.from({length: nPins}, (_, i) => ({
            id: data[3 + i * 2], pin: data[4 + i * 2]
          }));
          const iJack = data[3 + nPins * 2] ?? 0;
          return (
            <TreeNode label="MIDI OUT Jack Descriptor">
              <Leaf label="bJackType:" value={`${jackType(data[0])} (${hex2(data[0])})`} />
              <Leaf label="bJackID:" value={`${data[1]}`} />
              <Leaf label="bNrInputPins:" value={`${nPins}`} />
              {srcPairs.map((s, i) => <>
                <Leaf key={`id${i}`} label={`baSourceID[${i}]:`} value={`${s.id}`} />
                <Leaf key={`pin${i}`} label={`baSourcePin[${i}]:`} value={`${s.pin}`} />
              </>)}
              <Leaf label="iJack:" value={`${iJack}`} />
            </TreeNode>
          );
        }
      }

      // Generic audio fallback
      const audioSubtypes: Record<number, Record<number, string>> = {
        1: { 0x01:"AC_HEADER", 0x02:"INPUT_TERMINAL", 0x03:"OUTPUT_TERMINAL", 0x04:"MIXER_UNIT", 0x06:"FEATURE_UNIT" },
        2: { 0x01:"AS_GENERAL", 0x02:"FORMAT_TYPE" },
        3: { 0x01:"MS_HEADER", 0x02:"MIDI_IN_JACK", 0x03:"MIDI_OUT_JACK" },
      };
      const subName = audioSubtypes[ifaceSubClass]?.[sub] ?? hex2(sub);
      return (
        <TreeNode label={`Audio Class-Specific Descriptor (${subName})`}>
          <Leaf label="data:" value={rawHex(data)} />
        </TreeNode>
      );
    }

    // Generic CS_INTERFACE fallback
    return (
      <TreeNode label={`Class-Specific Interface Descriptor (subtype ${hex2(sub)})`}>
        <Leaf label="data:" value={data.map(b => b.toString(16).padStart(2,"0")).join(" ")} />
      </TreeNode>
    );
  }

  // CS_ENDPOINT (0x25)
  if (typ === 0x25) {
    // MIDI Streaming MS_GENERAL (subclass 3, subtype 1)
    if (ifaceSubClass === 0x03 && sub === 0x01 && data.length >= 1) {
      const n = data[0];
      const jacks = data.slice(1, 1 + n);
      return (
        <TreeNode label="MIDI Streaming Data Endpoint Descriptor">
          <Leaf label="bNumEmbMIDIJack:" value={`${n}`} />
          {Array.from(jacks).map((j, i) => <Leaf key={i} label={`baAssocJackID[${i}]:`} value={`${j}`} />)}
        </TreeNode>
      );
    }
    return (
      <TreeNode label={`Class-Specific Endpoint Descriptor (subtype ${hex2(sub)})`}>
        <Leaf label="data:" value={data.map(b => b.toString(16).padStart(2,"0")).join(" ")} />
      </TreeNode>
    );
  }

  return null;
}

function InterfaceNode({
  iface,
  device,
  hid,
}: {
  iface: InterfaceDescriptor;
  device: UsbDevice_Serialize;
  hid: HidInterface_Serialize | undefined;
}) {
  const ifaceString = getString(device, iface.i_interface);
  const label = `Interface #${iface.b_interface_number} - ${interfaceClassLabel(iface.b_interface_class, iface.b_interface_sub_class)}`;
  const value = ifaceString ? `"${ifaceString}"` : undefined;

  return (
    <TreeNode label={label} value={value}>
      <Leaf label="Alternate Setting" value={`${iface.b_alternate_setting}`} />
      <Leaf label="Number of Endpoints" value={`${iface.endpoints.length}`} />
      <Leaf label="Interface Class:" value={`${iface.b_interface_class}   (${deviceClassLabel(iface.b_interface_class)})`} />
      <Leaf label="Interface Subclass;" value={(() => {
        const sub = iface.b_interface_sub_class;
        const name = interfaceSubclassLabel(iface.b_interface_class, sub);
        return name ? `${sub}   (${name})` : `${sub}`;
      })()} />
      <Leaf label="Interface Protocol:" value={`${iface.b_interface_protocol}`} />
      {(iface.class_descriptors ?? []).map((cs, i) => (
        <ClassSpecificNode key={i} cs={cs} ifaceClass={iface.b_interface_class} ifaceSubClass={iface.b_interface_sub_class} />
      ))}
      {hid && <HidDescriptorNode hid={hid} />}
      {iface.endpoints.map((ep, i) => (
        <EndpointNode key={i} ep={ep} />
      ))}
    </TreeNode>
  );
}

function IadNode({ iad, device }: { iad: IadDescriptor; device: UsbDevice_Serialize }) {
  const className = deviceClassLabel(iad.b_function_class);
  const name = getString(device, iad.i_function);
  const value = name ? `${className}   "${name}"` : className;
  return (
    <TreeNode label="Interface Association" value={value}>
      <Leaf label="First Interface" value={`${iad.b_first_interface}`} />
      <Leaf label="Interface Count" value={`${iad.b_interface_count}`} />
      <Leaf label="Function Class" value={`${iad.b_function_class}   (${className})`} />
      <Leaf label="Function Subclass" value={`${iad.b_function_sub_class}`} />
      <Leaf label="Interface Protocol" value={`${iad.b_function_protocol}`} />
      <Leaf label="Function String" value={name ? `${iad.i_function}   "${name}"` : `${iad.i_function} (none)`} />
    </TreeNode>
  );
}

function ConfigNode({
  cfg,
  device,
}: {
  cfg: ConfigDescriptor_Serialize;
  device: UsbDevice_Serialize;
}) {
  // Assign HID interfaces to HID-class (3) USB interfaces in order of appearance
  const hidClassIfaces = cfg.interfaces.filter((i) => i.b_interface_class === 3);

  return (
    <TreeNode label="Configuration Descriptor (current config)" defaultOpen>
      {cfg.raw_bytes && cfg.raw_bytes.length > 0 && (
        <TreeNode label={`Length (and contents):   ${cfg.raw_bytes.length}`}>
          <HexBlock bytes={cfg.raw_bytes} />
        </TreeNode>
      )}
      <Leaf label="Number of Interfaces:" value={`${cfg.interfaces.length}`} />
      <Leaf label="Configuration Value:" value={`${cfg.b_configuration_value}`} />
      <Leaf label="Attributes:" value={attrsLabel(cfg.bm_attributes)} />
      <Leaf label="MaxPower:" value={`${cfg.b_max_power * 2} mA`} />
      {cfg.interfaces.map((iface, i) => {
        const iad = (cfg.interface_associations ?? []).find(
          a => a.b_first_interface === iface.b_interface_number
        );
        const hidIdx = hidClassIfaces.indexOf(iface);
        const hid = hidIdx >= 0 ? device.hid_interfaces[hidIdx] : undefined;
        return (
          <React.Fragment key={i}>
            {iad && <IadNode iad={iad} device={device} />}
            <InterfaceNode iface={iface} device={device} hid={hid} />
          </React.Fragment>
        );
      })}
    </TreeNode>
  );
}

function DeviceDescriptorNode({ device, defaultOpen = false }: { device: UsbDevice_Serialize; defaultOpen?: boolean }) {
  const dd = device.device_descriptor;
  const mfr = getString(device, dd.i_manufacturer);
  const product = getString(device, dd.i_product);
  const serial = getString(device, dd.i_serial_number);

  return (
    <TreeNode label="Device Descriptor" defaultOpen={defaultOpen}>
      <Leaf label="Descriptor Version Number:" value={`0x${hex4(dd.bcd_usb)}`} />
      <Leaf label="Device Class:" value={`${dd.b_device_class}   (${deviceClassLabel(dd.b_device_class)})`} />
      <Leaf label="Device Subclass:" value={`${dd.b_device_sub_class}`} />
      <Leaf label="Device Protocol:" value={`${dd.b_device_protocol}`} />
      <Leaf label="Device MaxPacketSize:" value={`${dd.b_max_packet_size0}`} />
      <Leaf label="Device VendorID/ProductID:" value={`0x${hex4(dd.id_vendor)}/0x${hex4(dd.id_product)}`} />
      <Leaf label="Device Version Number:" value={`0x${hex4(dd.bcd_device)}`} />
      <Leaf label="Number of Configurations:" value={`${dd.b_num_configurations}`} />
      <Leaf label="Manufacturer String:" value={dd.i_manufacturer === 0 ? "0 (none)" : `${dd.i_manufacturer} "${mfr ?? ""}"`} />
      <Leaf label="Product String:" value={dd.i_product === 0 ? "0 (none)" : `${dd.i_product} "${product ?? ""}"`} />
      <Leaf label="Serial Number String:" value={dd.i_serial_number === 0 ? "0 (none)" : `${dd.i_serial_number} "${serial ?? ""}"`} />
      {dd.raw_bytes && dd.raw_bytes.length > 0 && (
        <TreeNode label="Raw Descriptor (hex):">
          <HexBlock bytes={dd.raw_bytes} />
        </TreeNode>
      )}
    </TreeNode>
  );
}

function DeviceNode({ device }: { device: UsbDevice_Serialize }) {
  const dd = device.device_descriptor;
  const product = getString(device, dd.i_product) ?? "(no product)";
  const typeLabel =
    dd.b_device_class === 9 ? "Hub device"
    : dd.b_device_class === 0 ? "Composite device"
    : `${deviceClassLabel(dd.b_device_class)} device`;

  const label = `${hex4(dd.id_vendor)}:${hex4(dd.id_product)} ${speedLabel(device.speed)} device @ ${lastPort(device)} (0x${device.location_id})`;
  const value = `${typeLabel}: "${product}"`;

  return (
    <TreeNode label={label} value={value}>
      <TreeNode label="Number Of Endpoints (includes EP0):">
        {device.configurations.map((cfg, i) => {
          const total = cfg.interfaces.reduce((sum, iface) => sum + iface.endpoints.length, 0) + 1;
          return (
            <Leaf key={i}
              label={`Total Endpoints for Configuration ${cfg.b_configuration_value} (current):`}
              value={`${total}`}
            />
          );
        })}
      </TreeNode>
      <DeviceDescriptorNode device={device} />
      {device.configurations.map((cfg, i) => (
        <ConfigNode key={i} cfg={cfg} device={device} />
      ))}
    </TreeNode>
  );
}

// ── Split view ────────────────────────────────────────────────────────────────

function DeviceListItem({
  device,
  selected,
  onClick,
}: {
  device: UsbDevice_Serialize;
  selected: boolean;
  onClick: () => void;
}) {
  const dd = device.device_descriptor;
  const product = getString(device, dd.i_product) ?? "(no product)";
  return (
    <div className={`device-item${selected ? " selected" : ""}`} onClick={onClick}>
      <span className="di-speed">{speedLabel(device.speed)}</span>
      <span className="di-vidpid">{hex4(dd.id_vendor)}:{hex4(dd.id_product)}</span>
      <span className="di-product">{product}</span>
    </div>
  );
}

type SplitTab = "device" | "config" | "hid";

function SplitView({ devices }: { devices: UsbDevice_Serialize[] }) {
  const [selected, setSelected] = useState<UsbDevice_Serialize | null>(null);
  const [tab, setTab] = useState<SplitTab>("device");

  const hasHid = (selected?.hid_interfaces.length ?? 0) > 0;

  function selectDevice(d: UsbDevice_Serialize) {
    setSelected(d);
    // If current tab is "hid" but the new device has no HID interfaces, fall back
    if (tab === "hid" && d.hid_interfaces.length === 0) setTab("device");
  }

  return (
    <div className="split-view">
      <div className="split-left">
        {devices.length === 0 && <div className="empty">No USB devices found.</div>}
        {devices.map((d) => (
          <DeviceListItem
            key={d.location_id}
            device={d}
            selected={selected?.location_id === d.location_id}
            onClick={() => selectDevice(d)}
          />
        ))}
      </div>
      <div className="split-divider" />
      <div className="split-right">
        {selected ? (
          <>
            <div className="tab-bar">
              <button className={`tab${tab === "device" ? " active" : ""}`} onClick={() => setTab("device")}>Device Descriptor</button>
              <button className={`tab${tab === "config" ? " active" : ""}`} onClick={() => setTab("config")}>Configuration</button>
              {hasHid && <button className={`tab${tab === "hid" ? " active" : ""}`} onClick={() => setTab("hid")}>HID</button>}
            </div>
            <div className="tab-content">
              <DepthCtx.Provider value={0}>
                {tab === "device" && <DeviceDescriptorNode device={selected} defaultOpen />}
                {tab === "config" && selected.configurations.map((cfg, i) => (
                  <ConfigNode key={i} cfg={cfg} device={selected} />
                ))}
                {tab === "hid" && selected.hid_interfaces.map((hid, i) => (
                  <HidDescriptorNode key={i} hid={hid} />
                ))}
              </DepthCtx.Provider>
            </div>
          </>
        ) : (
          <div className="no-selection">← Select a device</div>
        )}
      </div>
    </div>
  );
}

// ── App ───────────────────────────────────────────────────────────────────────

type View = "tree" | "split";

export default function App() {
  const [devices, setDevices] = useState<UsbDevice_Serialize[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [view, setView] = useState<View>("tree");
  const [autoRefresh, setAutoRefresh] = useState(false);

  async function saveOutput() {
    let path = await save({ defaultPath: "usb-devices.txt" });
    if (!path) return;
    if (!path.endsWith(".txt")) path += ".txt";
    const text = await commands.formatAsText(devices);
    await commands.writeTextFile(path, text);
  }

  async function saveJson() {
    let path = await save({ defaultPath: "usb-devices.json" });
    if (!path) return;
    if (!path.endsWith(".json")) path += ".json";
    await commands.writeTextFile(path, JSON.stringify(devices, null, 2));
  }

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

  // Keep stable refs so the menu listener (registered once) always calls
  // the current versions of these functions without stale closures.
  const saveOutputRef = useRef(saveOutput);
  const saveJsonRef = useRef(saveJson);
  const refreshRef = useRef(refresh);
  useEffect(() => {
    saveOutputRef.current = saveOutput;
    saveJsonRef.current = saveJson;
    refreshRef.current = refresh;
  });

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<string>("menu-action", (event) => {
      switch (event.payload) {
        case "save_text":  saveOutputRef.current(); break;
        case "save_json":  saveJsonRef.current(); break;
        case "refresh":    refreshRef.current(); break;
        case "view_tree":  setView("tree"); break;
        case "view_split": setView("split"); break;
      }
    }).then(f => { unlisten = f; });
    return () => { unlisten?.(); };
  }, []);

  useEffect(() => {
    if (!autoRefresh) return;
    let unlisten: (() => void) | undefined;
    listen("usb-changed", () => { refreshRef.current(); })
      .then(f => { unlisten = f; });
    return () => { unlisten?.(); };
  }, [autoRefresh]);

  return (
    <div className="app">
      <header>
        <div className="header-start">
          <h1>USB Probester</h1>
          <button onClick={refresh} disabled={loading}>
            {loading ? "Refreshing…" : "Refresh"}
          </button>
          <label className="auto-toggle">
            <input type="checkbox" checked={autoRefresh} onChange={e => setAutoRefresh(e.target.checked)} />
            <span className="auto-toggle-track" />
            Auto
          </label>
        </div>
        <div className="header-mid">
          <button onClick={saveOutput} disabled={devices.length === 0}>Save Output</button>
          <button onClick={saveJson} disabled={devices.length === 0}>Save JSON</button>
        </div>
        <div className="header-end">
          <div className="view-toggle">
            <button className={view === "tree" ? "active" : ""} onClick={() => setView("tree")}>Tree</button>
            <button className={view === "split" ? "active" : ""} onClick={() => setView("split")}>Split</button>
          </div>
        </div>
      </header>

      {error && <div className="error">{error}</div>}

      {view === "tree" ? (
        <DepthCtx.Provider value={0}>
          <div className="device-list">
            {devices.length === 0 && !loading && !error && (
              <div className="empty">No USB devices found.</div>
            )}
            {devices.map((d) => (
              <DeviceNode key={d.location_id} device={d} />
            ))}
          </div>
        </DepthCtx.Provider>
      ) : (
        <SplitView devices={devices} />
      )}
    </div>
  );
}
