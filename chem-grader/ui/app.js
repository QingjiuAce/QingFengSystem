/* ============================================
   清风AI系统 — 界面逻辑（空壳骨架）
   数据区域走空状态，交互点标 TODO，待接后端
   ============================================ */

/* ---------- Tauri 桥（浏览器中为 null，功能降级） ---------- */
const invoke = (window.__TAURI__ && window.__TAURI__.core) ? window.__TAURI__.core.invoke : null;

/* ---------- 主题 ---------- */
(function initTheme() {
  const saved = localStorage.getItem('chemTheme');
  const h = new Date().getHours();
  const theme = saved || ((h >= 6 && h < 18) ? 'light' : 'dark');
  document.documentElement.setAttribute('data-theme', theme);
})();

/* ---------- Toast ---------- */
let toastTimer = null;
function toast(msg) {
  const el = document.getElementById('toast');
  el.textContent = msg;
  el.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove('show'), 2200);
}

/* ---------- 页面切换 ---------- */
function showPage(name) {
  document.querySelectorAll('.page').forEach(p => p.classList.remove('active'));
  document.getElementById('page-' + name).classList.add('active');
  document.querySelectorAll('.nav-item[data-page]').forEach(n => {
    n.classList.toggle('active', n.dataset.page === name);
  });
}
document.querySelectorAll('.nav-item[data-page]').forEach(item => {
  item.addEventListener('click', () => showPage(item.dataset.page));
});

/* ---------- 数据区域：空状态 ---------- */

// 成绩结果表
function renderTable(students) {
  const body = document.getElementById('resultBody');
  if (!students || !students.length) {
    body.innerHTML = '<tr><td colspan="7" class="empty" style="border:none;">还没有批改结果</td></tr>';
    return;
  }
  // TODO: 按后端返回的成绩数据渲染
  body.innerHTML = '';
}

// 成绩总览
function renderOverview() {
  document.getElementById('statGrid').innerHTML = ['班级平均分', '最高分', '最低分', '待复核']
    .map(label => '<div class="stat-card"><div class="stat-label">' + label + '</div><div class="stat-value">—</div></div>')
    .join('');
  document.getElementById('topicList').innerHTML = '<div class="empty">暂无数据</div>';
}

// 待复核
function renderReview() {
  document.getElementById('reviewList').innerHTML = '<div class="empty">暂无待复核项</div>';
  document.getElementById('reviewCount').textContent = '0 项待处理';
  const badge = document.getElementById('navBadge');
  if (badge) badge.hidden = true;
}

// 答案录入：渲染解析出的题目；缺分值/题型的打「需确认」
const Q_TYPES = ['选择题', '填空题', '化学方程式', '计算题', '实验题', '推断题', '有机合成', '简答题'];

// 答案框自适应高度（浏览器不支持 field-sizing 时用 JS 兜底）
const supportsFieldSizing = typeof CSS !== 'undefined' && CSS.supports && CSS.supports('field-sizing', 'content');
function autoGrow(el) {
  if (supportsFieldSizing) return;
  el.style.height = 'auto';
  el.style.height = el.scrollHeight + 'px';
}
function renderQ(questions) {
  const list = document.getElementById('qList');
  // 记住哪些行的「评分细则 / 原文对照」是展开的 —— 批量设分、加删题后的重渲染
  // 不该把老师正在看的对照内容又收起来
  const openIdx = new Set();
  Array.from(list.querySelectorAll('.q-block')).forEach((b, i) => {
    const d = b.querySelector('.q-detail');
    if (d && d.open) openIdx.add(i);
  });
  if (!questions || !questions.length) {
    list.innerHTML = '<div class="empty">尚未导入答案</div>';
    document.getElementById('needConfirm').textContent = '0 题需确认';
    document.getElementById('answerSummary').textContent = '尚未导入答案';
    return;
  }
  list.innerHTML = questions.map((q, i) => {
    const noScore = (q.score === null || q.score === undefined);
    const noType = !q.type;
    const noAnswer = !q.answer;
    const dual = q.agree === false;   // 双读不符
    const unver = !!q.unverified;     // 答案在原文里找不到出处
    // 严重度拆分：hard = 答案本身有问题（红）；soft = 只是信息待补（琥珀，不报警）
    const hard = unver || dual || noAnswer;
    const soft = !hard && (noScore || noType);
    const opts = '<option value=""' + (noType ? ' selected' : '') + '>—</option>' +
      Q_TYPES.map(t => '<option value="' + t + '"' + (t === q.type ? ' selected' : '') + '>' + t + '</option>').join('');
    const score = noScore ? '' : q.score;
    // 评分细则（可编辑；先前在前端被硬写成 null 丢掉了）+ 原文 / 第二读 对照
    const rub = '<div class="q-rub"><b>评分细则</b>' +
      '<textarea class="q-rubric" rows="1" placeholder="原文没有，可手动补录">' +
      esc(q.rubric || '') + '</textarea></div>';
    const snips =
      (q.src ? '<div class="q-snip"><b>原文</b>' + esc(q.src) + '</div>' : '') +
      (q.cross_src ? '<div class="q-snip"><b>第二读</b>' + esc(q.cross_src) + '</div>' : '');
    const detail = '<details class="q-detail"><summary>评分细则 / 原文对照</summary>' + rub + snips + '</details>';
    // 把"复核元数据"（双读/无出处/原文片段）藏进 data-* ——
    // 否则 collectQuestions→renderQ 重渲染后，这些标记会全部丢失
    const meta =
      ' data-agree="' + (q.agree === false ? 'no' : (q.agree === true ? 'yes' : '')) + '"' +
      ' data-unverified="' + (q.unverified ? '1' : '0') + '"' +
      ' data-src="' + esc(q.src || '') + '"' +
      ' data-cross="' + esc(q.cross_src || '') + '"';
    return '<div class="q-block">' +
      '<div class="q-item' + (hard ? ' flag' : (soft ? ' soft' : '')) + '"' + meta + '>' +
      // 显式空串要保留为空（老师清空准备重填时，别用序号又给它填回去）
      '<input class="q-no" value="' + esc(q.no === null || q.no === undefined ? (i + 1) : q.no) + '" title="题号可直接编辑">' +
      '<textarea class="q-answer" rows="1">' + esc(q.answer || '') + '</textarea>' +
      // step=0.01：化学卷常见 0.5 / 1.5 分/空，默认 step=1 会挡住小数
      '<input class="q-score" type="number" step="0.01" min="0" value="' + score + '">' +
      '<select>' + opts + '</select>' +
      '<div class="q-flagcell">' +
        (unver ? '<span class="dual">原文无出处</span><br>' : '') +
        (dual ? '<span class="dual">双读不符</span><br>' : '') +
        (noAnswer && !unver
          ? '<span class="dual" title="原文里没读到答案，常见原因是答案以公式图 / 结构式 / 手写呈现">无答案</span><br>'
          : '') +
        (soft ? '<span class="need">待补全</span>' : '') +
        '<button class="q-del" data-act="del" title="删除这一题">×</button>' +
      '</div>' +
      '</div>' + detail +
      '</div>';
  }).join('');
  // 恢复之前展开的对照内容（重渲染不该把老师正在看的内容收起来）
  if (openIdx.size) {
    Array.from(list.querySelectorAll('.q-block')).forEach((b, i) => {
      const d = b.querySelector('.q-detail');
      if (d && openIdx.has(i)) d.open = true;
    });
  }
  list.querySelectorAll('.q-answer, .q-rubric').forEach(el => {
    autoGrow(el);
    el.addEventListener('input', () => autoGrow(el));
  });
  const isHard = q => !!(q.unverified || q.agree === false || !q.answer);
  const isSoft = q => !isHard(q) && ((q.score === null || q.score === undefined) || !q.type);
  const hardN = questions.filter(isHard).length;
  const softN = questions.filter(isSoft).length;
  const duals = questions.filter(q => q.agree === false).length;
  const unvers = questions.filter(q => q.unverified).length;
  document.getElementById('needConfirm').textContent =
    hardN + ' 题需确认' +
    (unvers ? ' · ' + unvers + ' 题答案无出处' : '') +
    (duals ? ' · ' + duals + ' 题双读不符' : '') +
    (softN ? ' · ' + softN + ' 题待补全' : '');
  document.getElementById('answerSummary').textContent = '共 ' + questions.length + ' 题';
}

let currentQuestions = []; // 当前解析出的题目（供保存）
let currentImportPaths = []; // 本次导入的答案文件路径（保存时用于「绑定到本场考试」）
let answersDirty = false;  // 核对页是否有未保存的改动（重导前据它拦一下）

// 事件委托：核对表里任何编辑都标记为"未保存"。用委托是因为 renderQ 会整体重建行
(function trackAnswersDirty() {
  const list = document.getElementById('qList');
  if (!list) return;
  list.addEventListener('input', () => { answersDirty = true; });
  list.addEventListener('change', () => { answersDirty = true; });
})();

/* ---------- 导入进度 ---------- */
const importProgress = document.getElementById('importProgress');
const importProgressLabel = document.getElementById('importProgressLabel');
const importProgressBar = document.getElementById('importProgressBar');

function showImportProgress(label, pct) {
  importProgress.classList.add('show');
  if (label !== undefined) importProgressLabel.textContent = label;
  if (pct !== undefined) {
    importProgressBar.style.width = Math.max(0, Math.min(100, pct)) + '%';
  }
}
function hideImportProgress() { importProgress.classList.remove('show'); }

if (invoke && window.__TAURI__.event) {
  window.__TAURI__.event.listen('import-progress', (e) => {
    const p = e.payload || {};
    const total = p.total || 1;
    const pages = p.pages || 0;
    const page = p.page || 0;
    let msg, sub;
    if (p.stage === 'structure') {
      sub = 0.9;
      msg = '结构化 ' + p.index + '/' + total + ' · ' + p.file;
    } else if (p.stage === 'convert') {
      sub = 0.02;
      msg = '转换中（Word → PDF）· ' + p.file;
    } else if (pages > 0) {
      // 页级进度：识别占该文件进度条的前 80%
      sub = 0.8 * (page / pages);
      msg = '识别 第 ' + page + '/' + pages + ' 页 · ' + p.file;
    } else {
      sub = 0;
      msg = '识别 ' + p.index + '/' + total + ' · ' + p.file;
    }
    const frac = ((p.index - 1) + sub) / total;
    showImportProgress(msg, frac * 100);
  });

  // 答题卡扫描件：按份上报
  window.__TAURI__.event.listen('sheet-progress', (e) => {
    const p = e.payload || {};
    const total = p.total || 1;
    const i = p.index || 0;
    showImportProgress('答题卡 ' + i + '/' + total + ' · ' + p.file, (i / total) * 100);
  });
}

/* ---------- 收集核对后的题目（保存用） ---------- */
function collectQuestions() {
  return Array.from(document.querySelectorAll('#qList .q-item')).map(el => {
    const raw = el.querySelector('.q-score').value.trim();
    // 评分细则在 .q-item 的兄弟节点（.q-block 里），要从父元素取
    const block = el.parentElement;
    const rubEl = block ? block.querySelector('.q-rubric') : null;
    const ans = el.querySelector('.q-answer').value;
    return {
      no: el.querySelector('.q-no').value.trim(),
      answer: ans.trim() === '' ? null : ans,
      score: raw === '' ? null : Number(raw),
      type: el.querySelector('select').value || null,
      rubric: rubEl && rubEl.value.trim() !== '' ? rubEl.value.trim() : null,
      // 复核元数据从 data-* 取回，别在重渲染时丢
      agree: el.getAttribute('data-agree') === 'no' ? false
        : (el.getAttribute('data-agree') === 'yes' ? true : null),
      unverified: el.getAttribute('data-unverified') === '1',
      src: el.getAttribute('data-src') || null,
      cross_src: el.getAttribute('data-cross') || null
    };
  });
  // ⚠️ 不在这里过滤空题号的行 —— 老师可能只是想改题号（清空再重打），
  // 过滤掉会让这一行在下次重渲染时**静默消失**。空题号留给保存时校验。
}

/* ---------- 批量设分值 / 新增题目 / 行内删除 ---------- */
// 这些操作会就地改 DOM 再重渲染，所以 answersDirty 由调用方显式维护
function applyBatchScore(onlyEmpty) {
  const input = document.getElementById('batchScoreVal');
  const val = input.value.trim();
  if (val === '') { toast('请先填写要批量设置的分值'); return; }
  let n = 0;
  document.querySelectorAll('#qList .q-item').forEach(item => {
    const inp = item.querySelector('.q-score');
    if (onlyEmpty && inp.value.trim() !== '') return;
    inp.value = val;
    n++;
  });
  if (n === 0) { toast(onlyEmpty ? '没有空分值的题目' : '还没有题目可设置'); return; }
  answersDirty = true;
  renderQ(collectQuestions());
  toast('已为 ' + n + ' 题设置 ' + val + ' 分');
}

document.getElementById('batchScoreEmpty').addEventListener('click', () => applyBatchScore(true));
document.getElementById('batchScoreAll').addEventListener('click', () => applyBatchScore(false));

// 取"最大主号 + 1"作为新题号 —— 用 length+1 在删过题 / 题号不连续时会撞号
function nextQuestionNo(list) {
  let max = 0;
  list.forEach(q => {
    const m = String(q.no || '').match(/\d+/);
    if (m) {
      const n = parseInt(m[0], 10);
      if (!isNaN(n) && n > max) max = n;
    }
  });
  return String(max + 1);
}

document.getElementById('addQuestion').addEventListener('click', () => {
  const qs = collectQuestions();
  qs.push({
    no: nextQuestionNo(qs),
    answer: null,
    score: null,
    type: null,
    rubric: null
  });
  renderQ(qs);
  answersDirty = true;
  const items = document.querySelectorAll('#qList .q-item');
  const last = items[items.length - 1];
  if (last) {
    const noEl = last.querySelector('.q-no');
    if (noEl) noEl.focus();
  }
  toast('已新增一题，请填写题号与答案');
});

// 行内删除：事件委托（renderQ 会整体重建行）
document.getElementById('qList').addEventListener('click', (e) => {
  const btn = e.target.closest('.q-del');
  if (!btn) return;
  const block = btn.closest('.q-block');
  const noEl = block && block.querySelector('.q-no');
  const no = noEl ? noEl.value : '';
  if (!confirm('删除第 ' + no + ' 题？')) return;
  if (block) block.remove();
  answersDirty = true;
  renderQ(collectQuestions());
});

/* ---------- 答题卡扫描件：选文件夹 → 批量 OCR → 分类缓存并绑定 ---------- */
function pickSheetFolder(testId) {
  return new Promise(resolve => {
    const input = document.getElementById('sheetDirInput');
    if (!input) { toast('当前环境不支持选择文件夹'); resolve(); return; }
    const once = async () => {
      input.removeEventListener('change', once);
      const paths = Array.from(input.files || []).map(f => f.path).filter(Boolean);
      input.value = '';
      if (!paths.length) { toast('未取得文件路径（请在桌面版中操作）'); resolve(); return; }
      showImportProgress('准备识别答题卡…', 0);
      try {
        const out = await invoke('import_sheets', { testId: testId, paths: paths });
        hideImportProgress();
        toast('答题卡：共 ' + out.total + ' 份，已绑定 ' + out.bound + ' 份'
          + (out.reused ? '（复用缓存 ' + out.reused + ' 份）' : '')
          + (out.failed && out.failed.length ? '，失败 ' + out.failed.length + ' 份' : ''));
        if (out.failed && out.failed.length) renderImportNotice(out.failed, []);
      } catch (e) {
        hideImportProgress();
        toast('答题卡识别失败：' + e);
      }
      resolve();
    };
    input.addEventListener('change', once);
    input.click();
  });
}

/* ---------- 导入失败 / 降级提示 ---------- */
function renderImportNotice(failed, notes) {
  const el = document.getElementById('importNotice');
  const parts = [];
  if (failed && failed.length) {
    parts.push('以下 ' + failed.length + ' 份解析失败：' +
      failed.map(f => '<br>· ' + esc(String(f.path).split(/[\\/]/).pop()) + ' — ' + esc(f.error)).join(''));
  }
  if (notes && notes.length) {
    parts.push(notes.map(n => '· ' + esc(n)).join('<br>'));
  }
  if (!parts.length) { el.hidden = true; el.innerHTML = ''; return; }
  el.hidden = false;
  el.innerHTML = parts.join('<br><br>');
}

/* ---------- 缓存信息 / 清除 ---------- */
async function refreshCacheInfo() {
  if (!invoke) return;
  try {
    const s = await invoke('cache_stats', { task: currentTaskTitle || '未命名任务' });
    document.getElementById('cacheDir').value = s.dir;
    document.getElementById('cacheInfo').textContent =
      s.files + ' 个文件 · ' + (s.bytes / 1024).toFixed(1) + ' KB';
  } catch (e) { /* 忽略 */ }
}
document.getElementById('clearCache').addEventListener('click', async () => {
  if (!invoke) { toast('请在桌面版中使用'); return; }
  try {
    const n = await invoke('clear_cache', { task: currentTaskTitle || '未命名任务' });
    toast('已清除 ' + n + ' 个缓存文件');
    refreshCacheInfo();
  } catch (e) { toast('清除失败：' + e); }
});

/* ---------- 新建任务对话框 ---------- */
const newTaskDialog = document.getElementById('newTaskDialog');
const taskTitle = document.getElementById('taskTitle');
const taskErr = document.getElementById('taskErr');
let currentTaskTitle = ''; // 暂存标题：确认时不落库，等真正开始任务再建记录

function openNewTask() {
  taskTitle.value = '';
  taskErr.textContent = '';
  newTaskDialog.classList.add('show');
  newTaskDialog.setAttribute('aria-hidden', 'false');
  setTimeout(() => taskTitle.focus(), 60);
}
function closeNewTask() {
  newTaskDialog.classList.remove('show');
  newTaskDialog.setAttribute('aria-hidden', 'true');
}

document.getElementById('newTask').addEventListener('click', openNewTask);
document.getElementById('taskCancel').addEventListener('click', closeNewTask);
newTaskDialog.querySelector('.dialog-mask').addEventListener('click', closeNewTask);
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && newTaskDialog.classList.contains('show')) closeNewTask();
});
taskTitle.addEventListener('input', () => { if (taskErr.textContent) taskErr.textContent = ''; });
document.getElementById('taskConfirm').addEventListener('click', () => {
  const title = taskTitle.value.trim();
  if (!title) { taskErr.textContent = '请输入任务标题'; taskTitle.focus(); return; }
  // 只暂存标题，不立即创建记录
  currentTaskTitle = title;
  closeNewTask();
  // 第一个框淡出后，再弹「选择答案文件」
  setTimeout(openAnswerDialog, 200);
});

/* ---------- 选择答案文件对话框 ---------- */
const answerDialog = document.getElementById('answerDialog');
const fileInput = document.getElementById('fileInput');
const fileList = document.getElementById('fileList');
const fileErr = document.getElementById('fileErr');
let selectedFiles = []; // 按选择顺序保存 { name, path, mode }

const IMG_EXT = ['png', 'jpg', 'jpeg', 'bmp', 'webp', 'gif', 'tif', 'tiff'];
// 自动识别处理方式：图片走 OCR，Word / PDF 直接解析
function detectMode(name) {
  const ext = String(name).split('.').pop().toLowerCase();
  return IMG_EXT.includes(ext) ? 'ocr' : 'parse';
}
function esc(s) {
  return String(s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
function renderFiles() {
  if (!selectedFiles.length) {
    fileList.innerHTML = '<li class="file-empty">尚未选择文件</li>';
    return;
  }
  fileList.innerHTML = selectedFiles.map((f, i) => {
    const badge = f.mode === 'ocr'
      ? '<span class="kind kind-ocr">OCR</span>'
      : '<span class="kind kind-parse">解析</span>';
    return '<li class="file-item"><span class="idx">' + (i + 1) + '</span>' +
      '<span class="name">' + esc(f.name) + '</span>' + badge +
      '<button class="rm" data-i="' + i + '" title="移除">×</button></li>';
  }).join('');
  fileList.querySelectorAll('.rm').forEach(btn => btn.addEventListener('click', () => {
    selectedFiles.splice(Number(btn.dataset.i), 1);
    renderFiles();
  }));
}
function openAnswerDialog() {
  selectedFiles = [];
  fileInput.value = '';
  fileErr.textContent = '';
  renderFiles();
  answerDialog.classList.add('show');
  answerDialog.setAttribute('aria-hidden', 'false');
}
function closeAnswerDialog() {
  answerDialog.classList.remove('show');
  answerDialog.setAttribute('aria-hidden', 'true');
}

// 浏览器分支：原生 <input type=file>（拿不到绝对路径，只有文件名）
fileInput.addEventListener('change', () => {
  for (const f of fileInput.files) selectedFiles.push({ name: f.name, path: null, mode: detectMode(f.name) });
  fileInput.value = '';
  fileErr.textContent = '';
  renderFiles();
});

// 「选择文件」：Tauri 中走原生对话框（一次选一个，保证选择顺序）；否则退化为文件输入
document.getElementById('pickFiles').addEventListener('click', async () => {
  const dlg = window.__TAURI__ && window.__TAURI__.dialog;
  if (dlg) {
    const path = await dlg.open({
      multiple: false,
      title: '选择答案文件',
      filters: [
        { name: '答案文件', extensions: ['doc', 'docx', 'pdf', 'png', 'jpg', 'jpeg', 'bmp', 'webp', 'tif', 'tiff'] },
        { name: 'Word / PDF', extensions: ['doc', 'docx', 'pdf'] },
        { name: '图片', extensions: ['png', 'jpg', 'jpeg', 'bmp', 'webp', 'tif', 'tiff'] }
      ]
    });
    if (path) {
      const name = String(path).split(/[\\/]/).pop();
      selectedFiles.push({ name: name, path: String(path), mode: detectMode(name) });
      fileErr.textContent = '';
      renderFiles();
    }
  } else {
    fileInput.click();
  }
});
document.getElementById('fileConfirm').addEventListener('click', async () => {
  if (!selectedFiles.length) { fileErr.textContent = '请至少选择一个答案文件'; return; }
  if (!invoke) { fileErr.textContent = '文件解析需在桌面版中使用'; return; }
  const paths = selectedFiles.map(f => f.path).filter(Boolean);
  if (!paths.length) { fileErr.textContent = '未取得文件路径，请在桌面版中选择'; return; }
  // ② 防手滑：核对页有未保存改动时先确认，避免重新导入把手工修改无声覆盖
  if (answersDirty && !confirm('核对页有尚未保存的修改，重新导入会覆盖它们。确定继续吗？')) return;
  closeAnswerDialog();
  showImportProgress('准备解析…', 0);
  const forceEl = document.getElementById('forceReocr');
  const preferEl = document.getElementById('preferNew');
  try {
    const out = await invoke('import_answer_files', {
      task: currentTaskTitle || '未命名任务',
      paths: paths,
      force: !!(forceEl && forceEl.checked),
      preferNew: !!(preferEl && preferEl.checked)
    });
    hideImportProgress();
    currentQuestions = out.questions || [];
    currentImportPaths = paths.slice();
    renderQ(currentQuestions);
    answersDirty = false; // 刚导入，与数据源一致
    renderImportNotice(out.failed, out.notes);
    showPage('answers');
    const duals = currentQuestions.filter(q => q.agree === false).length;
    if (out.failed && out.failed.length) {
      toast('完成：' + currentQuestions.length + ' 题；' + out.failed.length + ' 份失败');
    } else if (duals) {
      toast('解析完成：' + currentQuestions.length + ' 题，其中 ' + duals + ' 题双读不符');
    } else {
      toast('解析完成：共 ' + currentQuestions.length + ' 题，请核对');
    }
    refreshCacheInfo();
  } catch (e) {
    hideImportProgress();
    toast('解析失败：' + e);
  }
});
document.getElementById('fileCancel').addEventListener('click', closeAnswerDialog);
answerDialog.querySelector('.dialog-mask').addEventListener('click', closeAnswerDialog);
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && answerDialog.classList.contains('show')) closeAnswerDialog();
});

/* ---------- 设置：API 配置 ---------- */
let appConfig = null; // 缓存后端配置

async function loadConfig() {
  if (!invoke) return;
  try {
    appConfig = await invoke('get_config');
    document.getElementById('keyZhipu').value = appConfig.ocr.api_key || '';
    document.getElementById('keyQwen').value = (appConfig.ocr2 && appConfig.ocr2.api_key) || '';
    document.getElementById('keyDeepseek').value = appConfig.llm.api_key || '';
  } catch (e) { /* 首次无配置文件时忽略 */ }
}
document.getElementById('saveKeys').addEventListener('click', async () => {
  if (!invoke) { toast('请在桌面版中保存'); return; }
  const cfg = appConfig || {
    ocr: { base_url: 'https://open.bigmodel.cn/api/paas/v4', api_key: '', model: 'glm-ocr' },
    llm: { base_url: 'https://api.deepseek.com/v1', api_key: '', model: 'deepseek-chat' }
  };
  if (!cfg.ocr2) {
    cfg.ocr2 = { base_url: 'https://maas.qianwenaiapi.com/compatible-mode/v1', api_key: '', model: 'qwen-vl-ocr' };
  }
  cfg.ocr.api_key = document.getElementById('keyZhipu').value.trim();
  cfg.ocr2.api_key = document.getElementById('keyQwen').value.trim();
  cfg.llm.api_key = document.getElementById('keyDeepseek').value.trim();
  try {
    await invoke('save_config', { cfg: cfg });
    appConfig = cfg;
    toast('已保存');
  } catch (e) { toast('保存失败：' + e); }
});
document.getElementById('testLlm').addEventListener('click', async () => {
  if (!invoke) { toast('请在桌面版中测试'); return; }
  const box = document.getElementById('testResult');
  box.hidden = false;
  box.innerHTML = '<div class="test-line"><span class="test-name">测试中…</span></div>';
  try {
    const results = await invoke('test_providers');
    box.innerHTML = results.map(r =>
      '<div class="test-line">' +
        '<span class="test-dot ' + (r.ok ? 'ok' : 'fail') + '"></span>' +
        '<span class="test-name">' + esc(r.name) + '</span>' +
        '<span class="test-detail">' + esc(r.detail || (r.ok ? '正常' : '失败')) + '</span>' +
      '</div>'
    ).join('');
    const okCount = results.filter(r => r.ok).length;
    toast(okCount === results.length
      ? '全部正常（' + okCount + '/' + results.length + '）'
      : '部分失败（' + okCount + '/' + results.length + '）');
  } catch (e) {
    box.innerHTML = '<div class="test-line"><span class="test-detail">测试失败：' + esc(String(e)) + '</span></div>';
  }
});

/* ---------- 其他交互点（待接入后端 / 桌面壳） ---------- */
document.getElementById('backToGrading').addEventListener('click', () => showPage('grading'));
document.getElementById('ocrPrefill').addEventListener('click', () => {
  // TODO: 调 OCR 预填答案表单
  toast('待接入：OCR 预填');
});
document.getElementById('importDoc').addEventListener('click', () => {
  // TODO: 解析 Word / Excel 导入答案
  toast('待接入：Word / Excel 导入');
});
document.getElementById('confirmAnswers').addEventListener('click', async () => {
  if (!invoke) { toast('请在桌面版中保存'); return; }
  const questions = collectQuestions();
  if (!questions.length) { toast('还没有可保存的答案'); return; }
  // 题号不能为空（空的存进去没意义，也没法按题号匹配学生答案）
  const emptyNo = questions.filter(q => !q.no).length;
  if (emptyNo) { toast('有 ' + emptyNo + ' 题的题号为空，请补全后再保存'); return; }
  try {
    const testId = await invoke('save_answers', {
      task: currentTaskTitle || '未命名任务',
      questions: questions,
      paths: currentImportPaths
    });
    currentQuestions = questions;
    answersDirty = false; // 已落库，不再是"未保存改动"
    toast('已保存（test #' + testId + '）');
    showPage('grading');
    // 答案已保存并绑定 → 接着问要不要导入答题卡
    if (confirm('已保存，并把本次答案文件绑定到这场考试。\n\n现在选择本次考试的答题卡扫描件文件夹吗？\n（文件夹里放全班的扫描件）')) {
      await pickSheetFolder(testId);
    }
  } catch (e) {
    toast('保存失败：' + e);
  }
});
document.getElementById('exportBtn').addEventListener('click', () => {
  // TODO: 导出成绩 Excel
  toast('待接入：导出');
});

/* ---------- 设置：主题开关 ---------- */
document.addEventListener('DOMContentLoaded', () => {
  const tt = document.getElementById('theme-toggle');
  const tl = document.getElementById('theme-label');
  if (document.documentElement.getAttribute('data-theme') === 'dark') { tt.checked = true; tl.textContent = '深色模式'; }
  tt.addEventListener('change', () => {
    const theme = tt.checked ? 'dark' : 'light';
    document.documentElement.setAttribute('data-theme', theme);
    tl.textContent = tt.checked ? '深色模式' : '亮色模式';
    localStorage.setItem('chemTheme', theme);
  });
});

/* ---------- 初始化 ---------- */
renderTable(null);
renderOverview();
renderReview();
renderQ();

// 运行在 Tauri 中时，读取真实数据目录与版本号
if (invoke) {
  const chip = document.getElementById('appState');
  if (chip) chip.textContent = '已连接';
  invoke('db_status')
    .then(p => { document.getElementById('dataDir').value = p; })
    .catch(() => {});
  invoke('app_info')
    .then(info => { document.getElementById('buildTag').textContent = 'Build ' + info.version; })
    .catch(() => {});
  loadConfig();
  refreshCacheInfo();
}
