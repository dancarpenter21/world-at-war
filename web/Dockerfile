FROM node:22-bookworm-slim AS build
WORKDIR /web
COPY web/package.json ./
RUN npm install
COPY web ./
RUN npm run build

FROM nginx:1.27-alpine
COPY web/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /web/dist /usr/share/nginx/html
EXPOSE 80
