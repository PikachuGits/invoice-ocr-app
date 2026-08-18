import { useState, useEffect, useCallback } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

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

function InfoItem({ label, value }: { label: string; value: string }) {
  if (!value) return null;
  return (
    <div className="info-item">
      <span className="info-label">{label}</span>
      <span className="info-value">{value}</span>
    </div>
  );
}

function App() {
  const [result, setResult] = useState<InvoiceResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [progress, setProgress] = useState<ProgressPayload | null>(null);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);

  useEffect(() => {
    const unlisten = listen<ProgressPayload>("ocr-progress", (event) => {
      setProgress(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const selectAndRecognize = useCallback(async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [
          { name: "图片文件", extensions: ["jpg", "jpeg", "png", "bmp", "webp"] },
        ],
      });
      if (!selected) return;

      const filePath = typeof selected === "string" ? selected : String(selected);
      setSelectedFile(filePath);
      setResult(null);
      setLoading(true);
      setProgress({ status: "progress", message: "正在提交图片..." });

      const data: InvoiceResult = await invoke("recognize_invoice", {
        imagePath: filePath,
      });
      setResult(data);
    } catch (err) {
      console.error("识别失败:", err);
      // Error is already shown via progress event
    } finally {
      setLoading(false);
    }
  }, []);

  const retry = useCallback(() => {
    if (selectedFile) {
      setResult(null);
      setLoading(true);
      setProgress({ status: "progress", message: "正在重新提交..." });
      invoke<InvoiceResult>("recognize_invoice", { imagePath: selectedFile })
        .then((data) => setResult(data))
        .catch((err) => console.error("重试失败:", err))
        .finally(() => setLoading(false));
    }
  }, [selectedFile]);

  const wr = result?.words_result;

  // The backend returns eight aligned arrays (one per detail column), each
  // item carrying the same 1-based "row" number. Zip them by row instead of
  // splitting a single word.
  const DETAIL_FIELDS = [
    "CommodityName",
    "CommodityType",
    "CommodityUnit",
    "CommodityNum",
    "CommodityPrice",
    "CommodityAmount",
    "CommodityTaxRate",
    "CommodityTax",
  ] as const;

  const commodityCount =
    wr && Array.isArray(wr.CommodityName) ? wr.CommodityName.length : 0;

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
      <header className="app-header">
        <h1>📄 发票 OCR 识别</h1>
        <p>基于 PaddleOCR 的智能发票识别系统</p>
      </header>

      <main className="app-main">
        {/* 选择文件按钮 */}
        <div className="action-bar">
          <button
            className="btn btn-primary"
            onClick={selectAndRecognize}
            disabled={loading}
          >
            {loading ? "识别中..." : "📷 选择发票图片"}
          </button>
          {selectedFile && (
            <span className="file-path" title={selectedFile}>
              {selectedFile.split(/[/\\]/).pop()}
            </span>
          )}
        </div>

        {/* 进度显示 */}
        {progress && (
          <div className={`progress-bar progress-${progress.status}`}>
            {progress.status === "progress" && <span className="spinner" />}
            {progress.status === "success" && <span>✅</span>}
            {progress.status === "error" && <span>❌</span>}
            <span>{progress.message}</span>
            {progress.status === "error" && selectedFile && !loading && (
              <button className="btn btn-retry" onClick={retry}>
                🔄 重试
              </button>
            )}
          </div>
        )}

        {/* 发票信息展示 */}
        {wr && (
          <div className="result-panel">
            {/* 基础信息 */}
            <div className="section">
              <h3>📋 基础信息</h3>
              <div className="info-grid">
                <InfoItem label="发票代码" value={wr.InvoiceCode as string} />
                <InfoItem label="发票号码" value={wr.InvoiceNum as string} />
                <InfoItem label="开票日期" value={wr.InvoiceDate as string} />
                <InfoItem label="发票类型" value={wr.InvoiceType as string} />
                <InfoItem label="开票人" value={wr.NoteDrawer as string} />
                <InfoItem label="收款人" value={wr.Payee as string} />
                <InfoItem label="复核人" value={wr.Checker as string} />
                <InfoItem label="省份" value={wr.Province as string} />
              </div>
            </div>

            {/* 购买方信息 */}
            <div className="section">
              <h3>🛒 购买方</h3>
              <div className="info-grid">
                <InfoItem label="名称" value={wr.PurchaserName as string} />
                <InfoItem
                  label="纳税人识别号"
                  value={wr.PurchaserRegisterNum as string}
                />
                <InfoItem
                  label="地址电话"
                  value={wr.PurchaserAddress as string}
                />
                <InfoItem
                  label="开户行及账号"
                  value={wr.PurchaserBank as string}
                />
              </div>
            </div>

            {/* 销售方信息 */}
            <div className="section">
              <h3>🏪 销售方</h3>
              <div className="info-grid">
                <InfoItem label="名称" value={wr.SellerName as string} />
                <InfoItem
                  label="纳税人识别号"
                  value={wr.SellerRegisterNum as string}
                />
                <InfoItem
                  label="地址电话"
                  value={wr.SellerAddress as string}
                />
                <InfoItem
                  label="开户行及账号"
                  value={wr.SellerBank as string}
                />
              </div>
            </div>

            {/* 商品明细 */}
            {commodityCount > 0 && (
              <div className="section">
                <h3>📦 商品明细</h3>
                <div className="table-wrapper">
                  <table className="commodity-table">
                    <thead>
                      <tr>
                        <th>名称</th>
                        <th>规格型号</th>
                        <th>单位</th>
                        <th>数量</th>
                        <th>单价</th>
                        <th>金额</th>
                        <th>税率</th>
                        <th>税额</th>
                      </tr>
                    </thead>
                    <tbody>
                      {Array.from({ length: commodityCount }).map(
                        (_, rowIndex) => (
                          <tr key={rowIndex}>
                            {DETAIL_FIELDS.map((field) => (
                              <td key={field}>
                                {detailValue(field, rowIndex) || "-"}
                              </td>
                            ))}
                          </tr>
                        )
                      )}
                    </tbody>
                  </table>
                </div>
              </div>
            )}

            {/* 合计信息 */}
            <div className="section">
              <h3>💰 合计</h3>
              <div className="info-grid">
                <InfoItem label="合计金额" value={wr.TotalAmount as string} />
                <InfoItem label="合计税额" value={wr.TotalTax as string} />
                <InfoItem
                  label="价税合计大写"
                  value={wr.AmountInWords as string}
                />
                <InfoItem
                  label="价税合计小写"
                  value={wr.AmountInFigures as string}
                />
              </div>
            </div>

            {/* 其他信息 */}
            <div className="section">
              <h3>📝 其他信息</h3>
              <div className="info-grid">
                <InfoItem label="密码区" value={wr.Password as string} />
                <InfoItem label="联次" value={wr.SheetNum as string} />
                <InfoItem label="备注" value={wr.Remarks as string} />
              </div>
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

export default App;
