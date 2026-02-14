## Dropshare

Welcome to Dropshare, a program that acts parallel to my rshare program.

**Features**

* Communicate with any device on your tailscale network
* Made to route files, not download them
* Read feedback logs from both the client and host.
* Features https encryption, just like the previous rshare
* Discoverable by devices on your tailnet ONLY.

*Why is this any different from rshare?*

First off, back up. This program isn't made to just quickly share something to a friend. This program is made to stay running on the Pi, 
or another device, always ready to serve files between your devices. You are the **ONLY** person 

**You need an account with Tailscale.**

Instructions: 

1. Extract and run host program (use a Rasbpi). A config file called hconfig will be made. Edit this config to change the maximum file size.

2. Run the client app on your computer(s). This will generate a client config file, and close the app. Enter the tailscale IP of the host device. Enter the name of your client.
If you want to run the client app on your host, you can.
    Re-run the app. control+click on the link made in the terminal to auto-open the browser. 
3. Have fun.

**Note: I cannot make apple apps, so it may work on a Macbook, but it will not work on a phone.**