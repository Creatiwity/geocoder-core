FROM gcr.io/distroless/static-debian13
COPY --chmod=+x ./geocoder-api /app/
WORKDIR /app
ENV HOST=0.0.0.0
ENV PORT=3000
ENV GEOCODER_DATA_DIR=/data
EXPOSE 3000
ENTRYPOINT ["./geocoder-api"]
CMD ["serve"]
