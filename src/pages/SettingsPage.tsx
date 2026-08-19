import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Link } from "react-router-dom";
import Header from "../components/Header";

type Section = "api" | "pagination" | "about";

const PAGE_SIZE_OPTIONS = [10, 20, 50, 100];

function SaveMessage({ message }: { message: { type: "success" | "error"; text: string } | null }) {
  if (!message) return null;
  return (
    <div className={`progress-bar progress-${message.type}`}>
      <span>{message.text}</span>
    </div>
  );
}

function ApiConfigSection() {
  const [apiUrl, setApiUrl] = useState("");
  const [token, setToken] = useState("");
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);

  useEffect(() => {
    (async () => {
      try {
        const url = await invoke<string | null>("get_config_value", { key: "api_url" });
        const t = await invoke<string | null>("get_config_value", { key: "token" });
        if (url) setApiUrl(url);
        if (t) setToken(t);
      } catch (err) {
        console.error("加载配置失败:", err);
      }
    })();
  }, []);

  const save = async () => {
    setSaving(true);
    setMessage(null);
    try {
      if (apiUrl.trim()) {
        await invoke("set_config_value", { key: "api_url", value: apiUrl.trim() });
      }
      if (token.trim()) {
        await invoke("set_config_value", { key: "token", value: token.trim() });
      }
      setMessage({ type: "success", text: "API 配置已保存" });
    } catch (err) {
      setMessage({ type: "error", text: `保存失败: ${err}` });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="section">
      <h3>API 配置</h3>

      <div className="form-group">
        <label className="form-label">API 地址</label>
        <input
          className="form-input"
          type="text"
          placeholder="https://paddleocr.aistudio-app.com/api/v2/ocr/jobs"
          value={apiUrl}
          onChange={(e) => setApiUrl(e.target.value)}
        />
      </div>

      <div className="form-group">
        <label className="form-label">Token</label>
        <input
          className="form-input"
          type="password"
          placeholder="请输入 API Token"
          value={token}
          onChange={(e) => setToken(e.target.value)}
        />
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
                    基于 PaddleOCR 的发票识别工具，支持多图合并、PDF 识别与 Excel 导出。
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
