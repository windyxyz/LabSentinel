FROM python:3.12-slim
WORKDIR /app/server
# Linux 下用 CPU 版 onnxruntime（requirements 已按 sys_platform 分流）
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt -i https://pypi.tuna.tsinghua.edu.cn/simple
COPY app ./app
EXPOSE 8000
CMD ["python", "-m", "uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "8000"]
