import base64
import json
import urllib.request
import urllib.error

KEY_FILE = r"C:\Users\36833\Desktop\Demo\OCR API.txt"
IMG = r"C:\Users\36833\Desktop\Demo\Demo001.png"
BASE = "https://open.bigmodel.cn/api/paas/v4"

KEY = open(KEY_FILE, encoding="utf-8").read().strip()


def post(path, payload, auth="bearer", timeout=180):
    url = BASE + path
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    if auth == "bearer":
        req.add_header("Authorization", "Bearer " + KEY)
    else:
        req.add_header("Authorization", KEY)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        try:
            body = json.loads(body)
        except Exception:
            pass
        return e.code, body
    except Exception as e:
        return None, repr(e)


def show(tag, st, res, limit=1200):
    print("\n" + "=" * 60)
    print(tag, "-> HTTP", st)
    if isinstance(res, dict):
        txt = json.dumps(res, ensure_ascii=False, indent=2)
    else:
        txt = str(res)
    print(txt[:limit])


b64 = base64.b64encode(open(IMG, "rb").read()).decode("ascii")
data_uri = "data:image/png;base64," + b64
print("image bytes:", len(b64), "approx")

# Test 1: layout_parsing + base64 data URI
st, res = post("/layout_parsing", {"model": "glm-ocr", "file": data_uri})
show("T1 layout_parsing + base64(data-uri)", st, res)

# Test 2: layout_parsing + public URL (confirm endpoint alive)
st, res = post(
    "/layout_parsing",
    {
        "model": "glm-ocr",
        "file": "https://cdn.bigmodel.cn/static/logo/introduction.png",
    },
)
show("T2 layout_parsing + public URL", st, res, 800)

# Test 3: vision chat + base64 data URI (fallback path)
vision_prompt = (
    "这是一张高中化学答题卡的扫描图片。请逐小问识别第16题的手写答案，"
    "只输出JSON，键为题号如 \"16(1)\"，值为识别到的答案文本。"
)
for model in ["glm-4.6v", "glm-4v-flash", "glm-5.3-flash"]:
    st, res = post(
        "/chat/completions",
        {
            "model": model,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "image_url", "image_url": {"url": data_uri}},
                        {"type": "text", "text": vision_prompt},
                    ],
                }
            ],
        },
    )
    tag = "T3 vision chat + base64 [%s]" % model
    if isinstance(res, dict) and res.get("choices"):
        content = res["choices"][0]["message"]["content"]
        show(tag, st, {"content": content, "usage": res.get("usage")}, 1200)
        break
    else:
        show(tag, st, res, 400)
