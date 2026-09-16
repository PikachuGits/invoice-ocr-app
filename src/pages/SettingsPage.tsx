import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Link } from "react-router-dom";
import Header from "../components/Header";

type Section = "api" | "pagination" | "about";

const PAGE_SIZE_OPTIONS = [10, 20, 50, 100];

type OcrProvider = "paddleocr" | "scnet";

const SCNET_OCR_TYPES = [
  { value: "GENERAL", label: "通用文字识别" },
  { value: "VAT_INVOICE", label: "增值税发票" },
  { value: "TAXI_INVOICE", label: "出租车发票" },
  { value: "TRAIN_TICKET", label: "火车票" },
  { value: "AIRPORT_TICKET", label: "航空运输电子客票行程单" },
  { value: "ID_CARD", label: "居民身份证" },
  { value: "BANK_CARD", label: "银行卡" },
  { value: "BUSINESS_LICENSE", label: "营业执照" },
];

function SaveMessage({ message }: { message: { type: "success" | "error"; text: string } | null }) {
  if (!message) return null;
  return (
    <div className={`progress-bar progress-${message.type}`}>
      <span>{message.text}</span>
    </div>
  );
}

function ProviderSelector({ value, onChange }: { value: OcrProvider; onChange: (v: OcrProvider) => void }) {
  return (
    <div className="form-group">
      <label className="form-label">OCR 服务</label>
      <div style={{ display: "flex", gap: 12 }}>
        <button
          type="button"
          className={`btn ${value === "paddleocr" ? "btn-primary" : "btn-secondary"}`}
          onClick={() => onChange("paddleocr")}
          style={{ flex: 1, padding: "10px 16px" }}
        >
          PaddleOCR VL
        </button>
        <button
          type="button"
          className={`btn ${value === "scnet" ? "btn-primary" : "btn-secondary"}`}
          onClick={() => onChange("scnet")}
          style={{ flex: 1, padding: "10px 16px" }}
        >
          SCNet OCR
        </button>
      </div>
    </div>
  );
}

function ApiConfigSection() {
  const [provider, setProvider] = useState<OcrProvider>("paddleocr");
  const [paddleocrUrl, setPaddleocrUrl] = useState("https://paddleocr.aistudio-app.com/api/v2/ocr/jobs");
  const [paddleocrToken, setPaddleocrToken] = useState("");
  const [scnetUrl, setScnetUrl] = useState("https://api.scnet.cn/api/llm/v1/ocr/recognize");
  const [scnetToken, setScnetToken] = useState("");
  const [scnetOcrType, setScnetOcrType] = useState("GENERAL");
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);

  useEffect(() => {
    (async () => {
      try {
        const get = async (key: string) => {
          try { return await invoke<string | null>("get_config_value", { key }); } catch { return null; }
        };
        const p = await get("ocr_provider");
        if (p === "scnet") setProvider("scnet");
        const purl = await get("paddleocr_url");
        if (purl) setPaddleocrUrl(purl);
        const ptk = await get("paddleocr_token");
        if (ptk) setPaddleocrToken(ptk);
        const surl = await get("scnet_url");
        if (surl) setScnetUrl(surl);
        const stk = await get("scnet_token");
        if (stk) setScnetToken(stk);
        const sot = await get("scnet_ocr_type");
        if (sot) setScnetOcrType(sot);
      } catch (err) {
        console.error("加载配置失败:", err);
      }
    })();
  }, []);

  const save = async () => {
    setSaving(true);
    setMessage(null);
    try {
      await invoke("set_config_value", { key: "ocr_provider", value: provider });
      await invoke("set_config_value", { key: "paddleocr_url", value: paddleocrUrl.trim() });
      await invoke("set_config_value", { key: "paddleocr_token", value: paddleocrToken.trim() });
      await invoke("set_config_value", { key: "scnet_url", value: scnetUrl.trim() });
      await invoke("set_config_value", { key: "scnet_token", value: scnetToken.trim() });
      await invoke("set_config_value", { key: "scnet_ocr_type", value: scnetOcrType });
      setMessage({ type: "success", text: `已保存，当前使用 ${provider === "paddleocr" ? "PaddleOCR VL" : "SCNet OCR"}` });
    } catch (err) {
      setMessage({ type: "error", text: `保存失败: ${err}` });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="section">
      <h3>API 配置</h3>

      <ProviderSelector value={provider} onChange={setProvider} />

      {provider === "paddleocr" && (
        <>
          <div className="form-group">
            <label className="form-label">PaddleOCR API 地址</label>
            <input
              className="form-input"
              type="text"
              placeholder="https://paddleocr.aistudio-app.com/api/v2/ocr/jobs"
              value={paddleocrUrl}
              onChange={(e) => setPaddleocrUrl(e.target.value)}
            />
          </div>
          <div className="form-group">
            <label className="form-label">PaddleOCR Token</label>
            <input
              className="form-input"
              type="password"
              placeholder="请输入 PaddleOCR API Token"
              value={paddleocrToken}
              onChange={(e) => setPaddleocrToken(e.target.value)}
            />
          </div>
        </>
      )}

      {provider === "scnet" && (
        <>
          <div className="form-group">
            <label className="form-label">SCNet API 地址</label>
            <input
              className="form-input"
              type="text"
              placeholder="https://api.scnet.cn/api/llm/v1/ocr/recognize"
              value={scnetUrl}
              onChange={(e) => setScnetUrl(e.target.value)}
            />
          </div>
          <div className="form-group">
            <label className="form-label">SCNet API Key</label>
            <input
              className="form-input"
              type="password"
              placeholder="请输入 SCNet API Key"
              value={scnetToken}
              onChange={(e) => setScnetToken(e.target.value)}
            />
          </div>
          <div className="form-group">
            <label className="form-label">识别类型 (ocrType)</label>
            <select
              className="form-input"
              value={scnetOcrType}
              onChange={(e) => setScnetOcrType(e.target.value)}
            >
              {SCNET_OCR_TYPES.map((t) => (
                <option key={t.value} value={t.value}>
                  {t.label} ({t.value})
                </option>
              ))}
            </select>
          </div>
        </>
      )}

      <SaveMessage message={message} />

      <div className="form-actions">
        <button className="btn btn-primary" onClick={save} disabled={saving}>
          {saving ? "保存中..." : "保存配置"}
        </button>
      </div>
    </div>
  );
}

function PaginationConfigSection() {
  const [pageSize, setPageSize] = useState("20");
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);

  useEffect(() => {
    (async () => {
      try {
        const ps = await invoke<string | null>("get_config_value", { key: "page_size" });
        if (ps) setPageSize(ps);
      } catch (err) {
        console.error("加载配置失败:", err);
      }
    })();
  }, []);

  const save = async () => {
    setSaving(true);
    setMessage(null);
    try {
      if (pageSize.trim()) {
        await invoke("set_config_value", { key: "page_size", value: pageSize.trim() });
      }
      setMessage({ type: "success", text: "分页配置已保存" });
    } catch (err) {
      setMessage({ type: "error", text: `保存失败: ${err}` });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="section">
      <h3>分页配置</h3>

      <div className="form-group">
        <label className="form-label">列表每页显示条数</label>
        <select
          className="form-input"
          value={pageSize}
          onChange={(e) => setPageSize(e.target.value)}
        >
          {PAGE_SIZE_OPTIONS.map((n) => (
            <option key={n} value={n}>
              {n} 条
            </option>
          ))}
        </select>
      </div>

      <SaveMessage message={message} />

      <div className="form-actions">
        <button className="btn btn-primary" onClick={save} disabled={saving}>
          {saving ? "保存中..." : "保存配置"}
        </button>
      </div>
    </div>
  );
}

function SettingsPage() {
  const [section, setSection] = useState<Section>("api");

  const menuItems: { key: Section; label: string; icon: string }[] = [
    { key: "api", label: "API 配置", icon: "⚙" },
    { key: "pagination", label: "分页配置", icon: "📄" },
    { key: "about", label: "关于", icon: "ℹ" },
  ];

  return (
    <div className="app">
      <Header />

      <main className="app-main">
        <div className="toolbar toolbar-sticky">
          <Link to="/" className="btn btn-secondary">
            ← 返回首页
          </Link>
          <span className="list-total">系统设置</span>
        </div>

        <div className="settings-layout">
          <aside className="settings-sidebar">
            {menuItems.map((item) => (
              <button
                key={item.key}
                className={`settings-menu-item ${
                  section === item.key ? "settings-menu-active" : ""
                }`}
                onClick={() => setSection(item.key)}
              >
                <span className="settings-menu-icon">{item.icon}</span>
                {item.label}
              </button>
            ))}
          </aside>

          <div className="settings-content">
            {section === "api" ? (
              <ApiConfigSection />
            ) : section === "pagination" ? (
              <PaginationConfigSection />
            ) : (
              <div className="section">
                <h3>关于</h3>
                <div className="info-item">
                  <span className="info-label">应用名称</span>
                  <span className="info-value">发票 OCR 识别</span>
                </div>
                <div className="info-item">
                  <span className="info-label">版本</span>
                  <span className="info-value">1.0.0</span>
                </div>
                <div className="info-item">
                  <span className="info-label">说明</span>
                  <span className="info-value">
                    基于 PaddleOCR / SCNet OCR 的发票识别工具，支持多图合并、PDF 识别与 Excel 导出。
                  </span>
                </div>
              </div>
            )}
          </div>
        </div>
      </main>
    </div>
  );
}

export default SettingsPage;
