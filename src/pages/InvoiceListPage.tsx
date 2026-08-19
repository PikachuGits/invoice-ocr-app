import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useNavigate } from "react-router-dom";
import { Dayjs } from "dayjs";
import "dayjs/locale/zh-cn";
import { createTheme, ThemeProvider, useMediaQuery } from "@mui/material";
import { zhCN as muiZhCN } from "@mui/material/locale";
import Tabs from "@mui/material/Tabs";
import Tab from "@mui/material/Tab";
import { AdapterDayjs } from "@mui/x-date-pickers/AdapterDayjs";
import { LocalizationProvider } from "@mui/x-date-pickers/LocalizationProvider";
import { DatePicker } from "@mui/x-date-pickers/DatePicker";
import { zhCN } from "@mui/x-date-pickers/locales";
import Header from "../components/Header";
import RecognizeModal from "../components/RecognizeModal";

// 共享 MUI 主题（暗色跟随系统 + 中文）
function useMuiTheme() {
  const prefersDark = useMediaQuery("(prefers-color-scheme: dark)");
  return useMemo(
    () =>
      createTheme(
        {
          palette: { mode: prefersDark ? "dark" : "light" },
        },
        muiZhCN
      ),
    [prefersDark]
  );
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

interface InvoiceCounts {
  all: number;
  success: number;
  failed: number;
}

interface InvoiceListResponse {
  records: InvoiceRecord[];
  total: number;
  page: number;
  page_size: number;
  counts: InvoiceCounts;
}

interface ProgressPayload {
  status: "progress" | "success" | "error";
  message: string;
}

type TabKey = "all" | "success" | "failed";

const PAGE_SIZE_OPTIONS = [10, 20, 50, 100];

function getPageNumbers(current: number, total: number): (number | "...")[] {
  if (total <= 7) {
    return Array.from({ length: total }, (_, i) => i + 1);
  }
  const nums: (number | "...")[] = [1];
  if (current > 3) nums.push("...");
  const start = Math.max(2, current - 1);
  const end = Math.min(total - 1, current + 1);
  for (let i = start; i <= end; i++) nums.push(i);
  if (current < total - 2) nums.push("...");
  nums.push(total);
  return nums;
}

function TabBar({
  active,
  counts,
  onChange,
}: {
  active: TabKey;
  counts: InvoiceCounts;
  onChange: (t: TabKey) => void;
}) {
  const muiTheme = useMuiTheme();
  const tabs: { key: TabKey; label: string; count: number }[] = [
    { key: "all", label: "全部", count: counts.all },
    { key: "success", label: "识别成功", count: counts.success },
    { key: "failed", label: "识别失败", count: counts.failed },
  ];

  return (
    <ThemeProvider theme={muiTheme}>
      <Tabs
        value={active}
        onChange={(_e, v) => onChange(v as TabKey)}
        variant="fullWidth"
        sx={{
          bgcolor: "background.paper",
          borderRadius: 2,
          boxShadow: 1,
          mb: 2,
          position: "relative",
          "& .MuiTabs-flexContainer": {
            height: 48,
          },
          "& .MuiTabs-indicator": {
            height: "100%",
            top: 0,
            bottom: "auto",
            borderRadius: 2,
            background: "linear-gradient(135deg, #667eea 0%, #764ba2 100%)",
            zIndex: 0,
          },
          "& .MuiTab-root": {
            zIndex: 1,
            textTransform: "none",
            fontSize: 14,
            fontWeight: 500,
            minHeight: 48,
            color: "text.secondary",
            "&.Mui-selected": {
              color: "#fff",
              fontWeight: 600,
            },
          },
          "& .MuiTouchRipple-root": { display: "none" },
        }}
      >
        {tabs.map((t) => (
          <Tab
            key={t.key}
            value={t.key}
            label={
              <span className="tab-label-wrap">
                {t.label}
                <span className="tab-count">{t.count}</span>
              </span>
            }
          />
        ))}
      </Tabs>
    </ThemeProvider>
  );
}

function InvoiceListPage() {
  const navigate = useNavigate();
  const [data, setData] = useState<InvoiceListResponse | null>(null);
  const [page, setPage] = useState(1);
  const [jumpTo, setJumpTo] = useState("");
  const [dateFrom, setDateFrom] = useState<Dayjs | null>(null);
  const [dateTo, setDateTo] = useState<Dayjs | null>(null);
  const [pageSize, setPageSize] = useState(20);
  const [loading, setLoading] = useState(false);
  const [tab, setTab] = useState<TabKey>("all");
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [exportMode, setExportMode] = useState<"single_sheet" | "multi_sheet">("single_sheet");
  const [showExportModal, setShowExportModal] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [showRecognizeModal, setShowRecognizeModal] = useState(false);
  const [progress, setProgress] = useState<ProgressPayload | null>(null);

  // 请求序号，丢弃过期响应避免分页竞态
  const requestSeq = useRef(0);
  const checkAllRef = useRef<HTMLInputElement>(null);

  // 加载每页条数配置（设置页可改）
  useEffect(() => {
    invoke<string | null>("get_config_value", { key: "page_size" })
      .then((v) => {
        if (v) {
          const n = parseInt(v, 10);
          if (PAGE_SIZE_OPTIONS.includes(n)) setPageSize(n);
        }
      })
      .catch(() => {});
  }, []);

  const loadList = useCallback(
    async (p: number, t: TabKey, from?: Dayjs | null, to?: Dayjs | null, size?: number) => {
      const seq = ++requestSeq.current;
      setLoading(true);
      try {
        const res = await invoke<InvoiceListResponse>("get_invoice_list", {
          page: p,
          pageSize: size ?? pageSize,
          statusFilter: t === "all" ? null : t,
          startDate: from ? from.format("YYYY-MM-DD") : null,
          endDate: to ? to.format("YYYY-MM-DD") : null,
        });
        if (seq !== requestSeq.current) return;
        // 当前页无数据（例如记录被删除）时回退到上一页
        if (res.records.length === 0 && p > 1) {
          setPage(p - 1);
          return;
        }
        setData(res);
      } catch (err) {
        console.error("加载列表失败:", err);
      } finally {
        if (seq === requestSeq.current) setLoading(false);
      }
    },
    [pageSize]
  );

  useEffect(() => {
    loadList(page, tab, dateFrom, dateTo);
  }, [page, tab, dateFrom, dateTo, pageSize, loadList]);

  useEffect(() => {
    const unlisten = listen<ProgressPayload>("ocr-progress", (event) => {
      setProgress(event.payload);
      if (event.payload.status === "success") {
        loadList(page, tab, dateFrom, dateTo);
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [page, tab, dateFrom, dateTo, pageSize, loadList]);

  const totalPages = data ? Math.max(1, Math.ceil(data.total / pageSize)) : 1;
  const pageNumbers = useMemo(
    () => getPageNumbers(page, totalPages),
    [page, totalPages]
  );

  // 当前页数据范围（第几条到第几条 / 共几条）
  const totalCount = data?.total ?? 0;
  const rangeStart = totalCount === 0 ? 0 : (page - 1) * pageSize + 1;
  const rangeEnd = Math.min(page * pageSize, totalCount);

  const handlePageSizeChange = (size: number) => {
    setPageSize(size);
    setPage(1);
    setJumpTo("");
    loadList(1, tab, dateFrom, dateTo, size);
  };

  const handleJump = () => {
    const v = parseInt(jumpTo, 10);
    if (!Number.isNaN(v) && v >= 1 && v <= totalPages) {
      setPage(v);
    }
    setJumpTo("");
  };

  const switchTab = (t: TabKey) => {
    setTab(t);
    setPage(1);
    setSelected(new Set());
  };

  const handleCheck = (id: number) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // 全选仅针对当前页；翻页后选择保留（跨页记忆）
  const handleCheckAll = () => {
    if (!data || data.records.length === 0) return;
    const pageIds = new Set(data.records.map((r) => r.id));
    if (allCurrentSelected) {
      // 已全选当前页 → 取消当前页选择（保留其他页）
      setSelected((prev) => new Set([...prev].filter((id) => !pageIds.has(id))));
    } else {
      // 选中当前页全部（追加，不清空其他页）
      setSelected((prev) => {
        const next = new Set(prev);
        pageIds.forEach((id) => next.add(id));
        return next;
      });
    }
  };

  const allCurrentSelected =
    data != null &&
    data.records.length > 0 &&
    data.records.every((r) => selected.has(r.id));
  const someCurrentSelected =
    data != null && data.records.some((r) => selected.has(r.id));

  // 半选状态：当前页部分选中（跨页选中保留）
  useEffect(() => {
    if (checkAllRef.current) {
      checkAllRef.current.indeterminate =
        someCurrentSelected && !allCurrentSelected;
    }
  }, [someCurrentSelected, allCurrentSelected]);

  const handleReRecognize = async (id: number) => {
    try {
      await invoke("re_recognize_invoice", { id });
      loadList(page, tab, dateFrom, dateTo);
    } catch (err) {
      console.error("重新识别失败:", err);
    }
  };

  const handleDelete = async () => {
    if (selected.size === 0) return;
    const ok = window.confirm(
      `确定删除选中的 ${selected.size} 条发票记录及其附件吗？此操作不可恢复。`
    );
    if (!ok) return;
    try {
      await invoke("delete_invoices", { ids: Array.from(selected) });
      setSelected(new Set());
      loadList(page, tab, dateFrom, dateTo);
    } catch (err) {
      alert(`删除失败: ${err}`);
    }
  };

  const handleExport = async () => {
    if (selected.size === 0) return;
    setExporting(true);
    try {
      const path = await invoke<string>("export_invoices_excel", {
        ids: Array.from(selected),
        exportMode,
      });
      alert(`导出成功: ${path}`);
      setShowExportModal(false);
    } catch (err) {
      alert(`导出失败: ${err}`);
    } finally {
      setExporting(false);
    }
  };

  const selectedSuccessCount = selected.size;

  const muiTheme = useMuiTheme();

  return (
    <div className="app">
      <Header onRecognize={() => setShowRecognizeModal(true)} />

      <main className="app-main">
        <div className="toolbar">
          <div className="toolbar-left">
            <span className="list-total">共 {data?.total ?? 0} 条记录</span>
            <button
              className="btn btn-small"
              onClick={() => loadList(page, tab, dateFrom, dateTo)}
              title="刷新列表"
            >
              刷新
            </button>
          </div>

          <div className="toolbar-filters">
            <ThemeProvider theme={muiTheme}>
              <LocalizationProvider
                dateAdapter={AdapterDayjs}
                adapterLocale="zh-cn"
                localeText={
                  zhCN.components.MuiLocalizationProvider.defaultProps.localeText
                }
              >
                <DatePicker
                  label="开始日期"
                  format="YYYY-MM-DD"
                  value={dateFrom}
                  onChange={(v) => {
                    setDateFrom(v);
                    setPage(1);
                  }}
                  slotProps={{ textField: { size: "small" } }}
                  sx={{ width: 160 }}
                />
                <span className="date-sep">至</span>
                <DatePicker
                  label="结束日期"
                  format="YYYY-MM-DD"
                  value={dateTo}
                  onChange={(v) => {
                    setDateTo(v);
                    setPage(1);
                  }}
                  slotProps={{ textField: { size: "small" } }}
                  sx={{ width: 160 }}
                />
              </LocalizationProvider>
            </ThemeProvider>
            {(dateFrom || dateTo) && (
              <button
                className="btn btn-small"
                onClick={() => {
                  setDateFrom(null);
                  setDateTo(null);
                  setPage(1);
                }}
              >
                清除
              </button>
            )}
          </div>

          <div className="toolbar-actions">
            {selected.size > 0 && (
              <button className="btn btn-danger" onClick={handleDelete}>
                删除选中 ({selected.size})
              </button>
            )}
            {selected.size > 0 && (
              <button
                className="btn btn-accent"
                onClick={() => setShowExportModal(true)}
              >
                导出 Excel ({selectedSuccessCount})
              </button>
            )}
          </div>
        </div>

        <TabBar active={tab} counts={data?.counts ?? { all: 0, success: 0, failed: 0 }} onChange={switchTab} />

        {progress && (
          <div className={`progress-bar progress-${progress.status}`}>
            {progress.status === "progress" && <span className="spinner" />}
            {progress.status === "success" && <span className="icon-check" />}
            {progress.status === "error" && <span className="icon-error" />}
            <span>{progress.message}</span>
          </div>
        )}

        {loading && (
          <div className="progress-bar progress-progress">
            <span className="spinner" />
            <span>加载中...</span>
          </div>
        )}

        {data && data.records.length === 0 && (
          <div className="empty-state">
            <p>暂无识别记录</p>
            <button
              className="btn btn-primary"
              onClick={() => setShowRecognizeModal(true)}
            >
              识别发票
            </button>
          </div>
        )}

        {data && data.records.length > 0 && (
          <>
            <div className="invoice-list">
              <table className="list-table">
                <thead>
                  <tr>
                    <th className="col-check">
                      <input
                        ref={checkAllRef}
                        type="checkbox"
                        checked={allCurrentSelected}
                        onChange={handleCheckAll}
                      />
                    </th>
                    <th>ID</th>
                    <th>发票号码</th>
                    <th>文件 / 附件</th>
                    <th>状态</th>
                    <th>识别次数</th>
                    <th>识别时间</th>
                    <th>操作</th>
                  </tr>
                </thead>
                <tbody>
                  {data.records.map((rec) => {
                    const isSelected = selected.has(rec.id);
                    return (
                      <tr
                        key={rec.id}
                        className={isSelected ? "row-selected" : ""}
                      >
                        <td className="col-check">
                          <input
                            type="checkbox"
                            checked={isSelected}
                            onChange={() => handleCheck(rec.id)}
                          />
                        </td>
                        <td>{rec.id}</td>
                        <td>
                          <span className="invoice-num">
                            {rec.invoice_num
                              ? `${rec.invoice_code} ${rec.invoice_num}`
                              : "（未识别号码）"}
                          </span>
                        </td>
                        <td className="cell-file" title={rec.file_name}>
                          {rec.file_name}
                          {rec.attachment_count > 1 && (
                            <span className="attachment-badge">
                              +{rec.attachment_count - 1}
                            </span>
                          )}
                        </td>
                        <td>
                          <span className={`status-badge status-${rec.status}`}>
                            {rec.status === "success" ? "成功" : "失败"}
                          </span>
                        </td>
                        <td>{rec.ocr_count}</td>
                        <td>{rec.created_at}</td>
                        <td className="col-actions">
                          <button
                            className="btn btn-small"
                            onClick={() => navigate(`/detail?id=${rec.id}`)}
                          >
                            查看
                          </button>
                          <button
                            className="btn btn-small btn-small-accent"
                            onClick={() => handleReRecognize(rec.id)}
                          >
                            重新识别
                          </button>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>

            <div className="pagination">
              <span className="page-info">
                {totalCount === 0
                  ? "共 0 条记录"
                  : `第 ${rangeStart}-${rangeEnd} 条 / 共 ${totalCount} 条`}
              </span>

              <div className="pagination-pages">
                <select
                  className="page-size-select"
                  value={pageSize}
                  onChange={(e) => handlePageSizeChange(Number(e.target.value))}
                  title="每页显示条数"
                >
                  {PAGE_SIZE_OPTIONS.map((n) => (
                    <option key={n} value={n}>
                      {n} 条/页
                    </option>
                  ))}
                </select>
                <button
                  className="btn btn-small"
                  disabled={page <= 1}
                  onClick={() => setPage(1)}
                >
                  首页
                </button>
                <button
                  className="btn btn-small"
                  disabled={page <= 1}
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                >
                  上一页
                </button>
                {pageNumbers.map((n, i) =>
                  n === "..." ? (
                    <span key={`e${i}`} className="page-ellipsis">
                      …
                    </span>
                  ) : (
                    <button
                      key={n}
                      className={`page-num ${n === page ? "page-active" : ""}`}
                      onClick={() => setPage(n)}
                    >
                      {n}
                    </button>
                  )
                )}
                <button
                  className="btn btn-small"
                  disabled={page >= totalPages}
                  onClick={() => setPage((p) => p + 1)}
                >
                  下一页
                </button>
                <button
                  className="btn btn-small"
                  disabled={page >= totalPages}
                  onClick={() => setPage(totalPages)}
                >
                  末页
                </button>
                <span className="page-info">
                  {page} / {totalPages} 页
                </span>
                <input
                  className="jump-input"
                  type="number"
                  min={1}
                  max={totalPages}
                  placeholder="页码"
                  value={jumpTo}
                  onChange={(e) => setJumpTo(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleJump();
                  }}
                />
                <button
                  className="btn btn-small"
                  disabled={totalPages <= 1}
                  onClick={handleJump}
                >
                  跳转
                </button>
              </div>
            </div>
          </>
        )}

        {/* 识别弹窗 */}
        {showRecognizeModal && (
          <RecognizeModal
            onClose={() => setShowRecognizeModal(false)}
            onSuccess={() => {
              setProgress({ status: "success", message: "识别完成!" });
              setPage(1);
              setTab("all");
              loadList(1, "all");
            }}
          />
        )}

        {/* 导出弹窗 */}
        {showExportModal && (
          <div className="modal-overlay" onClick={() => setShowExportModal(false)}>
            <div className="modal" onClick={(e) => e.stopPropagation()}>
              <h3>导出 Excel</h3>
              <p className="modal-desc">
                已选择 {selected.size} 条记录，导出后每张发票包含完整字段与商品明细，失败记录会标注状态。
              </p>
              <div className="form-group">
                <label className="form-label">导出模式</label>
                <div className="radio-group">
                  <label className="radio-item">
                    <input
                      type="radio"
                      name="exportMode"
                      value="single_sheet"
                      checked={exportMode === "single_sheet"}
                      onChange={() => setExportMode("single_sheet")}
                    />
                    <span>合并到同一 Sheet（完整详情依次排列）</span>
                  </label>
                  <label className="radio-item">
                    <input
                      type="radio"
                      name="exportMode"
                      value="multi_sheet"
                      checked={exportMode === "multi_sheet"}
                      onChange={() => setExportMode("multi_sheet")}
                    />
                    <span>每张发票一个 Sheet（完整信息 + 明细表）</span>
                  </label>
                </div>
              </div>
              <div className="modal-actions">
                <button
                  className="btn btn-primary"
                  onClick={handleExport}
                  disabled={exporting}
                >
                  {exporting ? "导出中..." : "确认导出"}
                </button>
                <button
                  className="btn btn-secondary"
                  onClick={() => setShowExportModal(false)}
                >
                  取消
                </button>
              </div>
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

export default InvoiceListPage;