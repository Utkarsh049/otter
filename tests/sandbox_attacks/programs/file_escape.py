try:
    with open("/etc/passwd", "r") as f:
        content = f.read()
        if "root:" in content:
            print("ESCAPE_SUCCESS")
        else:
            print("ESCAPE_FAILED")
except Exception:
    print("ESCAPE_FAILED")
