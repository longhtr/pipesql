# Optional oracle layer. Provision the pinned verification image first.
FROM pipesql-verification-rust:1.98.1-time
ARG TARGETARCH
RUN case "$TARGETARCH" in \
      arm64) wheel_url='https://files.pythonhosted.org/packages/50/8d/281f0f9b9376d4b7f146913b26fac0aa2829cd1ee7e997f53a27411bbb92/pyarrow-22.0.0-cp311-cp311-manylinux_2_28_aarch64.whl'; wheel_sha='b41f37cabfe2463232684de44bad753d6be08a7a072f6a83447eeaf0e4d2a215' ;; \
      amd64) wheel_url='https://files.pythonhosted.org/packages/f5/e5/53c0a1c428f0976bf22f513d79c73000926cb00b9c138d8e02daf2102e18/pyarrow-22.0.0-cp311-cp311-manylinux_2_28_x86_64.whl'; wheel_sha='35ad0f0378c9359b3f297299c3309778bb03b8612f987399a0333a560b43862d' ;; \
      *) exit 1 ;; \
    esac \
    && python3 -I -B -c 'import sys; assert sys.version_info[:2] == (3, 11)' \
    && curl --fail --location --retry 2 --max-time 180 "$wheel_url" -o /tmp/pyarrow.whl \
    && echo "$wheel_sha  /tmp/pyarrow.whl" | sha256sum --check - \
    && python3 -I -B -m zipfile -e /tmp/pyarrow.whl /usr/local/lib/python3.11/dist-packages \
    && python3 -I -B -c 'import pyarrow as pa; import pyarrow.parquet; assert pa.__version__ == "22.0.0"; assert pa.array([1, None]).to_pylist() == [1, None]' \
    && rm /tmp/pyarrow.whl
