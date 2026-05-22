package dev.evvie.waylandcraft.desktop;

import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.nio.ByteBuffer;

import org.apache.commons.codec.digest.DigestUtils;
import org.lwjgl.system.MemoryUtil;

import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.textures.GpuTexture;
import com.mojang.blaze3d.textures.TextureFormat;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.mixin.NativeImageMixin;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.AbstractTexture;
import net.minecraft.client.renderer.texture.TextureManager;
import net.minecraft.resources.Identifier;

public class DesktopIcon {

	public final String path;

	private WaylandCraft wlc;

	private IconImage image = null;
	private IconTexture texture = null;
	private final Identifier identifier;

	public DesktopIcon(String appId, String path) {
		this.path = path;
		this.identifier = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "icon_" + DigestUtils.sha1Hex(appId));
		this.wlc = WaylandCraft.instance;
	}

	private DesktopIcon(Identifier identifier, IconImage image) {
		this.path = null;
		this.identifier = identifier;
		this.image = image;
		this.wlc = WaylandCraft.instance;
	}

	// Build an icon from a packed-ARGB pixel array, as published by an X11 app
	// in its _NET_WM_ICON property (alpha in the high byte). NativeImage's RGBA
	// format is byte-order R,G,B,A, so the channels are unpacked accordingly.
	public static DesktopIcon fromArgb(String key, int width, int height, int[] argb) {
		ByteBuffer buf = ByteBuffer.allocateDirect(width * height * 4);
		for(int pixel : argb) {
			buf.put((byte) (pixel >> 16));
			buf.put((byte) (pixel >> 8));
			buf.put((byte) pixel);
			buf.put((byte) (pixel >> 24));
		}
		buf.flip(); // memAddress() is position-relative; rewind past the filled data
		long addr = MemoryUtil.memAddress(buf);

		Identifier identifier = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "window_icon_" + DigestUtils.sha1Hex(key));
		NativeImage image = NativeImageMixin.createImage(NativeImage.Format.RGBA, width, height, false, addr);
		return new DesktopIcon(identifier, IconImage.direct(image, buf));
	}
	
	public synchronized void preload() {
		if(image != null) return; // image already preloaded
		if(path == null) return;
		
		File file = new File(path);
		
		/* These "file type checks" are valid because according to the Icon Theme Specification
		 * the extension has to be one of ".png", ".xpm" and ".svg" (lowercase) and the extension
		 * signals what type of file we should expect.
		 */
		
		if(getExtension(file).equals("png")) {
			try {
				FileInputStream stream = new FileInputStream(file);
				this.image = IconImage.standard(NativeImage.read(stream));
			} catch(IOException e) {
				WaylandCraft.LOGGER.warn("Skipping desktop icon, not a valid PNG: {} ({})", path, e.getMessage());
			}
		}
		else if(getExtension(file).equals("svg")) {
			final int width = 128;
			final int height = 128;
			
			ByteBuffer buf = ByteBuffer.allocateDirect(width * height * 4);
			long addr = MemoryUtil.memAddress(buf);
			
			if(wlc.bridge.renderSVG(file, width, height, addr)) {
				this.image = IconImage.direct(NativeImageMixin.createImage(NativeImage.Format.RGBA, width, height, false, addr), buf);
			}
		}
	}
	
	public void upload() {
		if(texture != null) return; // image already uploaded
		
		if(image == null) {
			// When upload is called before preload, that necessary step is completed first.
			preload();
		}
		if(image == null) return;
		
		texture = new IconTexture(image);
		texture.upload();
		
		TextureManager textureManager = Minecraft.getInstance().getTextureManager();
		textureManager.register(identifier, texture);
	}
	
	public Identifier getTextureLocation() {
		this.upload();
		if(texture == null) return null;
		return identifier;
	}
	
	private String getExtension(File file) {
		String path = file.getAbsolutePath();
		int idx = path.lastIndexOf('.');
		if(idx < 0 || idx >= path.length() - 1) return "";
		
		return path.substring(idx + 1);
	}
	
	private static class IconImage {
		
		public NativeImage nativeImage;
		public boolean close;
		
		// This field holds the NativeImage backing data allocated during image preload (if any).
		// This has to be a field here because otherwise Java would garbage collect the ByteBuffer
		// before the data is uploaded to OpenGL causing a use-after-free bug.
		@SuppressWarnings("unused")
		private ByteBuffer backing;
		
		private IconImage(NativeImage nativeImage, ByteBuffer backing, boolean close) {
			this.nativeImage = nativeImage;
			this.backing = backing;
			this.close = close;
		}
		
		public static IconImage standard(NativeImage image) {
			return new IconImage(image, null, true);
		}
		
		public static IconImage direct(NativeImage image, ByteBuffer backing) {
			return new IconImage(image, backing, false);
		}
		
	}
	
	private static class IconTexture extends AbstractTexture {
		
		public final IconImage image;
		
		public IconTexture(IconImage image) {
			this.image = image;
		}
		
		public void upload() {
			NativeImage nativeImage = image.nativeImage;
			
			this.texture = RenderSystem.getDevice().createTexture("icon texture", GpuTexture.USAGE_TEXTURE_BINDING | GpuTexture.USAGE_COPY_DST, TextureFormat.RGBA8, nativeImage.getWidth(), nativeImage.getHeight(), 1, 1);
			RenderSystem.getDevice().createCommandEncoder().writeToTexture(this.texture, nativeImage);
			this.textureView = RenderSystem.getDevice().createTextureView(this.texture);
			
			if(image.close) nativeImage.close();
			
			image.backing = null; // Allow java to garbage collect the data now
		}
		
	}
	
}
