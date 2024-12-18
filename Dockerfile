FROM docker.io/library/python:3.12-slim-bookworm

WORKDIR /app
COPY requirements.txt .
RUN pip install -r requirements.txt

COPY bot.py .

CMD [ "python", "./bot.py" ]
