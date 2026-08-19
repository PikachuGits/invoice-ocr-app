import { useState, useEffect, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { Link, useSearchParams } from "react-router-dom";
import Header from "../components/Header";
import "../App.css";

interface RowItem {
  word: string;
  row: string;
}

interface InvoiceResult {
  log_id: string;
  words_result_num: number;
  words_result: Record<string, string | RowItem[]>;
}

interface ProgressPayload {
  status: "progress" | "success" | "error";
  message: string;
}

interface InvoiceRecord {
  id: number;
  invoice_code: string;
  invoice_num: string;
  file_name: string;
  status: string;
  retry_count: number;
  ocr_count: number;
  parsed_result: string;
  created_at: string;
  updated_at: string;
  attachment_count: number;
}

interface InvoiceFile {
  id: number;
  invoice_id: number;
  sha256: string;
  md5: string;
  file_name: string;
  file_path: string;
  ocr_raw_json: string;
  page_count: number;
  created_at: string;
}

interface InvoiceDetailResponse {
  invoice: InvoiceRecord;
  files: InvoiceFile[];
}

function InfoItem({ label, value }: { label: string; value: string }) {
  if (!value) return null;
  const cleaned = value.replace(/^[ⓧⓍ✖✕❌✘Ⓟ]+\s*/, "").trim();
  if (!cleaned) return null;
  return (
    <div className="info-item">
      <span className="info-label">{label}</span>
      <span className="info-value">{cleaned}</span>
    </div>
  );
}

const FIELD_GROUPS: { title: string; fields: [string, string][] }[] = [
  {
    title: "基础信息",
    fields: [
      ["InvoiceNum", "发票号码"],
      ["InvoiceNumConfirm", "发票号码(确认)"],
      ["InvoiceNumDigit", "发票号码(数字)"],
      ["InvoiceCode", "发票代码"],
      ["InvoiceCodeConfirm", "发票代码(确认)"],
      ["InvoiceDate", "开票日期"],
      ["InvoiceType", "发票类型"],
      ["Province", "省份"],
      ["City", "城市"],
      ["SheetNum", "联次"],
      ["ServiceType", "服务类型"],
      ["OnlinePay", "线上支付"],
      ["Agent", "是否代理"],
    ],
  },
  {
    title: "购买方",
    fields: [
      ["PurchaserName", "名称"],
      ["PurchaserRegisterNum", "纳税人识别号"],
      ["PurchaserAddress", "地址电话"],
      ["PurchaserBank", "开户行及账号"],
    ],
  },
  {
    title: "销售方",
    fields: [
      ["SellerName", "名称"],
      ["SellerRegisterNum", "纳税人识别号"],
      ["SellerAddress", "地址电话"],
      ["SellerBank", "开户行及账号"],
    ],
  },
  {
    title: "开票信息",
    fields: [
      ["NoteDrawer", "开票人"],
      ["Payee", "收款人"],
      ["Checker", "复核人"],
    ],
  },
  {
    title: "合计",
    fields: [
      ["TotalAmount", "合计金额"],
      ["TotalTax", "合计税额"],
      ["AmountInWords", "价税合计(大写)"],
      ["AmountInFigures", "价税合计(小写)"],
    ],
  },
  {
    title: "其他信息",
    fields: [
      ["Password", "密码区"],
      ["Remarks", "备注"],
    ],
  },
];

const DETAIL_FIELDS: { key: string; label: string }[] = [
  { key: "CommodityName", label: "名称" },
  { key: "CommodityType", label: "规格型号" },
  { key: "CommodityUnit", label: "单位" },
  { key: "CommodityNum", label: "数量" },
  { key: "CommodityPrice", label: "单价" },
  { key: "CommodityAmount", label: "金额" },
  { key: "CommodityTaxRate", label: "税率" },
  { key: "CommodityTax", label: "税额" },
  { key: "CommodityPlateNum", label: "车牌号" },
  { key: "CommodityVehicleType", label: "车辆类型" },
  { key: "CommodityStartDate", label: "开始日期" },
  { key: "CommodityEndDate", label: "结束日期" },
];

function DetailPage() {
  const [searchParams] = useSearchParams();
  const invoiceId = searchParams.get("id");

  const [result, setResult] = useState<InvoiceResult | null>(null);
  const [record, setRecord] = useState<InvoiceRecord | null>(null);
  const [files, setFiles] = useState<InvoiceFile[]>([]);
  const [loading, setLoading] = useState(false);
  const [progress, setProgress] = useState<ProgressPayload | null>(null);
  const [parseError, setParseError] = useState<string | null>(null);

  const loadRecord = useCallback(async (id: number) => {
    setLoading(true);
    setParseError(null);
    try {
      const detail = await invoke<InvoiceDetailResponse | null>(
        "get_invoice_detail",
        { id }
      );
      if (!detail) {
        setParseError("记录不存在");
        return;
      }
      const rec = detail.invoice;
      setRecord(rec);
      setFiles(detail.files);
      if (rec.parsed_result) {
        try {
          const parsed: InvoiceResult = JSON.parse(rec.parsed_result);
          setResult(parsed);
        } catch (e) {
          setResult(null);
          setParseError("识别结果解析失败");
        }
      } else {
        setResult(null);
      }
    } catch (err) {
      console.error("加载历史记录失败:", err);
      setParseError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (invoiceId) {
      loadRecord(Number(invoiceId));
    }
  }, [invoiceId, loadRecord]);

  useEffect(() => {
    const unlisten = listen<ProgressPayload>("ocr-progress", (event) => {
      setProgress(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const handleReRecognize = async () => {
    if (!record) return;
    setLoading(true);
    setProgress({ status: "progress", message: "正在重新识别..." });
    try {
      const data = await invoke<InvoiceResult>("re_recognize_invoice", {
        id: record.id,
      });
      setResult(data);
      await loadRecord(record.id);
    } catch (err) {
      console.error("重新识别失败:", err);
      await loadRecord(record.id);
    } finally {
      setLoading(false);
    }
  };

  const wr = result?.words_result;

  const commodityCount =
    wr && Array.isArray(wr.CommodityName) ? wr.CommodityName.length : 0;

  // 动态明细列：仅显示识别结果中实际有数据的字段（普通发票不含车牌号等运输字段）
  const visibleDetailFields = DETAIL_FIELDS.filter((f) => {
    const list = wr?.[f.key];
    return Array.isArray(list) && list.length > 0;
  });

  const detailValue = (field: string, rowIndex: number): string => {
    if (!wr) return "";
    const list = wr[field];
    if (!Array.isArray(list)) return "";
    const items = list as RowItem[];
    const byRow = items.find((item) => item.row === String(rowIndex + 1));
    return (byRow ?? items[rowIndex])?.word ?? "";
  };

  return (
    <div className="app">
      <Header />

      <main className="app-main">
        {/* 顶部工具条：返回 + 记录元信息 + 重新识别（吸顶） */}
        <div className="toolbar toolbar-sticky">
          <Link className="btn btn-secondary" to="/">
            返回列表
          </Link>
          {record && (
            <>
              <span
                className="file-path detail-file"
                title={record.file_name}
              >
                {record.invoice_num
                  ? `${record.invoice_code} ${record.invoice_num}`
                  : record.file_name}
              </span>
              <span className={`status-badge status-${record.status}`}>
                {record.status === "success" ? "识别成功" : "识别失败"}
              </span>
              <span className="detail-meta">
                {record.attachment_count} 个附件 · 识别 {record.ocr_count} 次 ·
                失败 {record.retry_count} 次 · {record.created_at}
              </span>
              <button
                className="btn btn-small-accent"
                onClick={handleReRecognize}
                disabled={loading}
              >
                重新识别
              </button>
            </>
          )}
        </div>

        {progress && (
          <div className={`progress-bar progress-${progress.status}`}>
            {progress.status === "progress" && <span className="spinner" />}
            <span>{progress.message}</span>
          </div>
        )}

        {loading && !result && (
          <div className="progress-bar progress-progress">
            <span className="spinner" />
            <span>{record ? "加载中..." : "识别中..."}</span>
          </div>
        )}

        {/* 失败状态 */}
        {record && record.status === "failed" && !result && !loading && (
          <div className="section">
            <h3>识别失败</h3>
            <p className="modal-desc">
              该发票此前识别失败（失败 {record.retry_count} 次）。请点击右上角
              "重新识别" 再次尝试，或先到设置页确认 API 与 Token 配置。
            </p>
          </div>
        )}

        {parseError && (
          <div className="progress-bar progress-error">
            <span>{parseError}</span>
          </div>
        )}

        {files.length > 0 && (
          <div className="section">
            <h3>附件 ({files.length})</h3>
            <div className="table-wrapper">
              <table className="commodity-table">
                <thead>
                  <tr>
                    <th>#</th>
                    <th>文件名</th>
                    <th>页数</th>
                    <th>识别时间</th>
                    <th>文件路径</th>
                  </tr>
                </thead>
                <tbody>
                  {files.map((f, i) => (
                    <tr key={f.id}>
                      <td>{i + 1}</td>
                      <td>{f.file_name}</td>
                      <td>{f.page_count}</td>
                      <td>{f.created_at}</td>
                      <td className="cell-path" title={f.file_path}>
                        {f.file_path}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        )}

        {wr && (
          <div className="result-panel">
            {FIELD_GROUPS.map((group) => (
              <div className="section" key={group.title}>
                <h3>{group.title}</h3>
                <div className="info-grid">
                  {group.fields.map(([key, label]) => (
                    <InfoItem key={key} label={label} value={wr[key] as string} />
                  ))}
                </div>
              </div>
            ))}

            <div className="section">
              <h3>商品明细</h3>
              {commodityCount > 0 && visibleDetailFields.length > 0 ? (
                <div className="table-wrapper">
                  <table className="commodity-table">
                    <thead>
                      <tr>
                        {visibleDetailFields.map((f) => (
                          <th key={f.key}>{f.label}</th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {Array.from({ length: commodityCount }).map(
                        (_, rowIndex) => (
                          <tr key={rowIndex}>
                            {visibleDetailFields.map((f) => (
                              <td key={f.key}>
                                {detailValue(f.key, rowIndex) || "-"}
                              </td>
                            ))}
                          </tr>
                        )
                      )}
                    </tbody>
                  </table>
                </div>
              ) : (
                <p className="modal-desc">（无商品明细）</p>
              )}
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

export default DetailPage;