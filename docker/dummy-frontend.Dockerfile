# syntax=docker/dockerfile:1.7
FROM nginx:1.31-alpine
COPY crates/dummy-frontend/templates/index.html.tmpl /etc/stubby/index.html.tmpl
COPY crates/dummy-frontend/templates/style.css       /etc/stubby/style.css
COPY crates/dummy-frontend/nginx/default.conf        /etc/nginx/conf.d/default.conf
COPY crates/dummy-frontend/nginx/entrypoint.sh       /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh
EXPOSE 80
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
