import base64
import json
import urllib.request

KEY_FILE = r"C:\Users\36833\Desktop\Demo\OCR API.txt"
IMG = r"C:\Users\36833\Desktop\Demo\Demo001.png"
BASE = "https://open.bigmodel.cn/api/paas/v4"
OUT = r"C:\Users\36833\WorkBuddy\QingFengSystem\chem-grader\p0\result_vision.json"

KEY = open(KEY_FILE, encoding="utf-8").read().strip()
b64 = base64.b64encode(open(IMG, "rb").read()).decode("ascii")

prompt = (
    "这是一张高中化学答题卡的扫描图片。请逐小问识别第16题的手写答案，"
    "只输出JSON，键为题号如 \"16(1)\"，值为识别到的答案文本。"
)
payload = {
    "model": "glm-4.6v",
    "messages": [
        {
            "role": "user",
            "content": [
                {"type": "image_url", "image_url": {"url": "data:image/png;base64," + b64}},
                {"type": "text", "text": prompt},
            ],
        }
    ],
}
req = urllib.request.Request(
    BASE + "/chat/completions", data=json.dumps(payload).encode("utf-8"), method="POST"
)
req.add_header("Content-Type", "application/json")
req.add_header("Authorization", "Bearer " + KEY)

with urllib.request.urlopen(req, timeout=180) as r:
    res = json.loads(r.read().decode("utf-8"))

json.dump(res, open(OUT, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
print("saved:", OUT)
print("usage:", json.dumps(res.get("usage"), ensure_ascii=False))
print("content:", res["choices"][0]["message"]["content"])
