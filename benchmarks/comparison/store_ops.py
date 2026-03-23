import struct
import os

HEADER_SIZE = 24
MAGIC = b"JADESTR\0"
FILENAME = "records_py.store"
REC_FMT = "qq"  # two int64s
REC_SIZE = struct.calcsize(REC_FMT)

def main():
    if os.path.exists(FILENAME):
        os.remove(FILENAME)

    fp = open(FILENAME, "w+b")
    fp.write(MAGIC)
    fp.write(struct.pack("q", 0))  # count
    fp.write(struct.pack("q", REC_SIZE))  # rec_size
    fp.flush()

    # Insert 10000 records
    for i in range(10000):
        fp.seek(8)
        count = struct.unpack("q", fp.read(8))[0]
        fp.seek(HEADER_SIZE + count * REC_SIZE)
        fp.write(struct.pack(REC_FMT, i, i * 7))
        count += 1
        fp.seek(8)
        fp.write(struct.pack("q", count))
        fp.flush()

    # Query 1000 times
    total = 0
    for j in range(1000):
        fp.seek(8)
        count = struct.unpack("q", fp.read(8))[0]
        fp.seek(HEADER_SIZE)
        for _ in range(count):
            data = fp.read(REC_SIZE)
            key, value = struct.unpack(REC_FMT, data)
            if key == j:
                total += value
                break
    print(total)

    # Count
    fp.seek(8)
    count = struct.unpack("q", fp.read(8))[0]
    print(count)

    fp.close()
    os.remove(FILENAME)

if __name__ == "__main__":
    main()
