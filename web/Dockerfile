FROM node:20-alpine AS build
WORKDIR /app
COPY package.json ./
RUN npm install --registry=https://registry.npmmirror.com
COPY . .
RUN npm run build

FROM nginx:alpine
COPY --from=build /app/dist /usr/share/nginx/html
# API/WS 反代到服务端
RUN printf 'server { listen 80; location / { root /usr/share/nginx/html; try_files $uri /index.html; } location /api/ { proxy_pass http://labsentinel:8000; } location /ws/ { proxy_pass http://labsentinel:8000; proxy_http_version 1.1; proxy_set_header Upgrade $http_upgrade; proxy_set_header Connection "upgrade"; } }' > /etc/nginx/conf.d/default.conf
EXPOSE 80
