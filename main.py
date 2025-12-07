
""" CANSAT PICO Emitter node (With ACK + Encryption) """

from machine import SPI, Pin
from rfm69 import RFM69
import time

led = Pin(25, Pin.OUT)

FREQ           = 433.1
ENCRYPTION_KEY = b"\x01\x02\x03\x04\x05\x06\x07\x08\x01\x02\x03\x04\x05\x06\x07\x08"
NODE_ID        = 120
BASESTATION_ID = 100

spi = SPI(0, miso=Pin(4), mosi=Pin(7), sck=Pin(6),
          baudrate=50000, polarity=0, phase=0, firstbit=SPI.MSB)
nss = Pin(5, Pin.OUT, value=True)
rst = Pin(3, Pin.OUT, value=False)

rfm = RFM69(spi=spi, nss=nss, reset=rst)
rfm.frequency_mhz = FREQ
rfm.tx_power = 20
rfm.encryption_key = ENCRYPTION_KEY   # 🔐 Encryption enabled (16 bytes)
rfm.node = NODE_ID
rfm.destination = BASESTATION_ID

print("Freq:", rfm.frequency_mhz)
print("NODE:", rfm.node)
print("DEST:", BASESTATION_ID)
print("Running WITH ACK + ENCRYPTION...")

counter = 1
last_rssi = None

rfm.ack_retries = 2   # Try up to x times for ACK
rfm.ack_wait = 1.0    # Wait x ms for ACK

# Read Local pressure then
# Calculate corresponding Altitude
#
from machine import I2C, Pin
# BME280 aslo work for BMP280
from bme280 import BME280, BMP280_I2CADDR
from time import sleep
i2c = I2C(0, sda=Pin(8), scl=Pin(9) )

baseline = 1032.0 # day's pressure at sea level
bmp = BME280( i2c=i2c, address=BMP280_I2CADDR )

while True:
    led.toggle()
    sensor_all = bmp.raw_values
    sensor_temperature = bmp.raw_values[0]
    sensor_pressure = bmp.raw_values[1]
    sensor_humidity = bmp.raw_values[2]
    altitude = (baseline - sensor_pressure)*8.3
    msg = " %d  %3.1f  %2.2f  %4.2f  %2.2f  %3.6f" % (counter, 0 if last_rssi is None else last_rssi, sensor_temperature, sensor_pressure, sensor_humidity, altitude)
    print("Send:", msg)
    ack = rfm.send_with_ack(bytes(msg, "utf-8"))
    if ack:
        last_rssi = rfm.rssi
        print("   +-> ACK received | RSSI (ACK): %3.1f dBm" % rfm.rssi)
    else:
        print("   +-> ACK missing")
    counter += 1
    time.sleep(0.1)  # send every x second

