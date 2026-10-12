import base64
import json
import urllib.request
import urllib.error

KEY_FILE = r"C:\Users\36833\Desktop\Demo\OCR API.txt"
IMG = r"C:\Users\36833\Desktop\Demo\Demo001.png"
BASE = "https://open.bigmodel.cn/api/paas/v4"
OUT = r"C:\Users\36833\WorkBuddy\QingFengSystem\chem-grader\p0\result_layout.json"

KEY = open(KEY_FILE, encoding="utf-8").read().strip()
b64 = base64.b64encode(open(IMG, "rb").read()).decode("ascii")

payload = {"model": "glm-ocr", "file": "data:image/png;base64," + b64}
req = urllib.request.Request(
    BASE + "/layout_parsing", data=json.dumps(payload).encode("utf-8"), method="POST"
)
req.add_header("Content-Type", "application/json")
req.add_header("Authorization", "Bearer " + KEY)

with urllib.request.urlopen(req, timeout=180) as r:
    res = json.loads(r.read().decode("utf-8"))

json.dump(res, open(OUT, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
print("saved:", OUT)
print("usage:", json.dumps(res.get("usage"), ensure_ascii=False))
print("md_results:\n" + (res.get("md_results") or "(none)"))
print("\n--- layout elements ---")
for page in res.get("layout_details", []):
    for el in page:
        print("[%s] %s" % (el.get("label"), el.get("content")))
